// ============================================================
// rt.c -- dacelo Gen 2 native runtime
//
// memory model : mark-sweep GC, conservative stack scanning
// values       : every value is a 64-bit word
//                int   : (n << 2) | 1          (odd)
//                bool  : false=3, true=7       (odd)
//                other : aligned pointer to heap block (even)
// heap block   : [header][payload...]
//                header = (size_words << 8) | (mark << 7) | tag
// ============================================================

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>

typedef uint64_t value;

#define MARK_BIT   0x0000000000000080ULL
#define HDR(tag, size) (((uint64_t)(size) << 8) | (uint64_t)(tag))
#define HDR_MARKED(hdr) ((hdr) & MARK_BIT)
#define SET_MARK(hdr)   ((hdr) | MARK_BIT)
#define CLR_MARK(hdr)   ((hdr) & ~MARK_BIT)
#define TAG_OF(hdr)     ((hdr) & 0x7F)
#define SIZE_OF(hdr)    ((hdr) >> 8)

enum {
    T_STRING  = 1,
    T_TUPLE   = 2,
    T_ADT     = 3,
    T_CLOSURE = 4,
    T_UNIT    = 5,
};

#define MK_INT(n)   ((((uint64_t)(intptr_t)(n)) << 2) | 1)
#define INT_OF(v)   (((int64_t)(v)) >> 2)
#define IS_INT(v)   (((v) & 3) == 1)
#define BOOL_FALSE  ((value)3)
#define BOOL_TRUE   ((value)7)
#define IS_BOOL(v)  (((v) & 3) == 3)

// static unit singleton; exported so generated code can load it directly
uint64_t dc_unit_block[1] = { HDR(T_UNIT, 1) };

// ------------------------------------------------------------------
// heap: chunk list + free list
// ------------------------------------------------------------------

typedef struct Chunk {
    struct Chunk *next;
    uint64_t *start;      // first block
    uint64_t *end;        // one past last word
} Chunk;

static Chunk  *chunks       = NULL;
static uint64_t *bump_ptr   = NULL;
static uint64_t *bump_end   = NULL;
static uint64_t  next_gc    = 8ull * 1024 * 1024;  // bytes before first GC
// DACELO_NO_GC=1 disables collection entirely (diagnosis / huge heaps)
static int gc_disabled(void) {
    static int v = -1;
    if (v < 0) v = getenv("DACELO_NO_GC") != NULL;
    return v;
}
static uint64_t  live_bytes = 0;

typedef struct FreeBlock {
    uint64_t hdr;              // tag 0, size = block size
    struct FreeBlock *next;
} FreeBlock;

static FreeBlock *freelist = NULL;

#define CHUNK_WORDS (1024 * 1024)   // 8 MB per chunk

static void add_chunk(void) {
    Chunk *c = (Chunk *)malloc(sizeof(Chunk));
    uint64_t *mem = (uint64_t *)malloc(CHUNK_WORDS * sizeof(uint64_t));
    if (!c || !mem) {
        fprintf(stderr, "dacelo: out of memory\n");
        exit(1);
    }
    c->start = mem;
    c->end   = mem + CHUNK_WORDS;
    c->next  = chunks;
    chunks   = c;
    bump_ptr = mem;
    bump_end = c->end;
}

// ------------------------------------------------------------------
// marking
// ------------------------------------------------------------------

static value *gc_stack_bottom = NULL;

static Chunk *chunk_of(uint64_t *p) {
    for (Chunk *c = chunks; c; c = c->next) {
        if (p >= c->start && p < c->end) return c;
    }
    return NULL;
}

static uint64_t **mark_stack = NULL;
static size_t mark_sp = 0, mark_cap = 0;

static void ms_push(uint64_t *p) {
    if (mark_sp == mark_cap) {
        mark_cap = mark_cap ? mark_cap * 2 : 4096;
        mark_stack = (uint64_t **)realloc(mark_stack, mark_cap * sizeof(uint64_t *));
        if (!mark_stack) { fprintf(stderr, "dacelo: gc oom\n"); exit(1); }
    }
    mark_stack[mark_sp++] = p;
}

static void mark_block(uint64_t *blk);

// A candidate found by the conservative scan may be stale debris (an old
// spill slot, C-call scratch, ...) pointing into the middle of a live
// object.  Blindly SET_MARK-ing such a word corrupts real data (e.g. a
// string length or a closure field gets +0x80).  Only accept candidates
// whose target word plausibly IS a block header.
static int hdr_plausible(uint64_t h) {
    if (TAG_OF(h) > 4) return 0;              // valid tags: 0..4
    uint64_t sz = SIZE_OF(h);
    return sz >= 2 && sz <= (64u << 20);      // sane block size
}

static void mark_word(value v) {
    if ((v & 7) != 0) return;                 // immediates are odd
    uint64_t *p = (uint64_t *)v;
    if (!chunk_of(p)) return;                 // not ours (statics, etc.)
    if (!hdr_plausible(*p)) return;           // interior pointer / debris
    if (HDR_MARKED(*p)) return;
    *p = SET_MARK(*p);
    ms_push(p);
}

static void mark_block(uint64_t *blk) {
    uint64_t hdr = CLR_MARK(*blk);
    uint64_t size = SIZE_OF(hdr);
    switch (TAG_OF(hdr)) {
        case T_STRING:
            break;                             // len + bytes: no pointers
        case T_TUPLE:
            for (uint64_t i = 0; i + 1 < size; i++) mark_word(blk[1 + i]);
            break;
        case T_ADT:
            for (uint64_t i = 0; i + 2 < size; i++) mark_word(blk[2 + i]);
            break;                             // blk[1] is raw ctor_id
        case T_CLOSURE:
            // blk[1]=code, blk[2]=env_size raw; env starts at blk[3]
            for (uint64_t i = 0; i + 3 < size; i++) mark_word(blk[3 + i]);
            break;
        case T_UNIT:
            break;
        default:
            break;
    }
}

extern uint64_t dc_global_table[];            // defined below
extern uint64_t dc_global_count;

static void gc(void) {
    mark_sp = 0;
    // clear marks
    for (Chunk *c = chunks; c; c = c->next) {
        uint64_t *p = c->start;
        while (p < c->end) {
            uint64_t hdr = *p;
            uint64_t sz = SIZE_OF(hdr);
            if (sz == 0) break;
            *p = CLR_MARK(hdr);
            p += sz;
        }
    }
    // roots: globals (precise)
    for (uint64_t i = 0; i < dc_global_count; i++) mark_word(dc_global_table[i]);
    // roots: conservative stack scan
    void *dummy;
    uint64_t *sp = (uint64_t *)&dummy;
    if (!gc_stack_bottom) gc_stack_bottom = sp;
    if (gc_stack_bottom < sp) { void *t = gc_stack_bottom; gc_stack_bottom = (void*)sp; sp = (uint64_t*)t; }
    for (uint64_t *w = sp; w < (uint64_t *)gc_stack_bottom; w++) {
        mark_word(*w);
    }
    // drain
    while (mark_sp > 0) {
        uint64_t *blk = mark_stack[--mark_sp];
        mark_block(blk);
    }
    // sweep: rebuild free list
    freelist = NULL;
    live_bytes = 0;
    for (Chunk *c = chunks; c; c = c->next) {
        uint64_t *p = c->start;
        while (p < c->end) {
            uint64_t hdr = *p;
            uint64_t sz = SIZE_OF(hdr);
            if (sz == 0) { p = c->end; break; }
            if (HDR_MARKED(hdr)) {
                *p = CLR_MARK(hdr);
                live_bytes += sz * 8;
            } else {
                FreeBlock *fb = (FreeBlock *)p;
                fb->hdr = HDR(0, sz);
                fb->next = freelist;
                freelist = fb;
            }
            p += sz;
        }
    }
}

// ------------------------------------------------------------------
// allocation
// ------------------------------------------------------------------

void *dacelo_alloc(uint64_t nbytes) {
    uint64_t words = (nbytes + 7) / 8;
    if (words < 2) words = 2;
    for (;;) {
        // first-fit free list
        FreeBlock **prev = &freelist;
        for (FreeBlock *fb = freelist; fb; prev = &fb->next, fb = fb->next) {
            uint64_t bsz = SIZE_OF(fb->hdr);
            if (bsz == words) {
                *prev = fb->next;
                return fb;
            }
            if (bsz > words + 1) {             // split, keep remainder >= 2 words
                uint64_t rem = bsz - words;
                FreeBlock *rest = (FreeBlock *)((uint64_t *)fb + words);
                rest->hdr = HDR(0, rem);
                rest->next = fb->next;
                *prev = rest;
                return fb;
            }
        }
        if (bump_ptr + words <= bump_end) {
            void *p = bump_ptr;
            bump_ptr += words;
            return p;
        }
        if (!gc_disabled() && live_bytes + nbytes > next_gc) {
            gc();
            next_gc = (live_bytes + nbytes) * 2;
            if (next_gc < 8ull * 1024 * 1024) next_gc = 8ull * 1024 * 1024;
            continue;
        }
        add_chunk();
    }
}

void *dc_alloc_bytes(uint64_t nbytes) { return dacelo_alloc(nbytes); }

// ------------------------------------------------------------------
// constructors for runtime values
// ------------------------------------------------------------------

static value make_string(const char *bytes, uint64_t len) {
    // reserve one spare byte so the content is ALWAYS NUL-terminated for
    // C interop (fopen/fprintf), even when len is a multiple of 8
    uint64_t words = 2 + (len + 8) / 8;
    uint64_t *blk = (uint64_t *)dacelo_alloc(words * 8);
    blk[0] = HDR(T_STRING, words);
    blk[1] = len;
    memcpy(&blk[2], bytes, len);
    memset((char *)&blk[2] + len, 0, words * 8 - 16 - len);
    return (value)blk;
}

#define STR_OF(v)   ((const char *)&((uint64_t *)(v))[2])
#define STRLEN_OF(v)(((uint64_t *)(v))[1])

static value make_tuple(uint64_t n, value *fields) {
    uint64_t *blk = (uint64_t *)dacelo_alloc((1 + n) * 8);
    blk[0] = HDR(T_TUPLE, 1 + n);
    memcpy(&blk[1], fields, n * 8);
    return (value)blk;
}

static value make_adt(uint64_t ctor_id, uint64_t n, value *fields) {
    uint64_t *blk = (uint64_t *)dacelo_alloc((2 + n) * 8);
    blk[0] = HDR(T_ADT, 2 + n);
    blk[1] = ctor_id;
    memcpy(&blk[2], fields, n * 8);
    return (value)blk;
}

value dacelo_make_bool(int b) { return b ? BOOL_TRUE : BOOL_FALSE; }

// ------------------------------------------------------------------
// global variable table
// ------------------------------------------------------------------

uint64_t dc_global_table[1 << 16];
uint64_t dc_global_count = 0;

void dc_gset(uint64_t i, value v) {
    dc_global_table[i] = v;
    if (i + 1 > dc_global_count) dc_global_count = i + 1;
}

value dc_gget(uint64_t i) {
    if (getenv("DACELO_TRACE_GGET") && i >= 40) {
        fprintf(stderr, "[gget %llu lr=%p]\n", (unsigned long long)i, (void*)__builtin_return_address(0));
    }
    return dc_global_table[i];
}

// ------------------------------------------------------------------
// constructor arity table + partial-application machinery
// ------------------------------------------------------------------

uint64_t dc_arity_table[4096];                // ctor_id -> field count
uint64_t dc_ctors_registered = 0;

void dc_register_ctor(uint64_t id, uint64_t arity) {
    dc_arity_table[id] = arity;
    if (id + 1 > dc_ctors_registered) dc_ctors_registered = id + 1;
}

value dc_nil_instance = 0;                    // set by init

// closure layout: [hdr][code][env_size][env...]
value dc_ctor_step(value clo, value arg);

static value mk_closure(void *code, uint64_t nenv, value *env) {
    uint64_t *blk = (uint64_t *)dacelo_alloc((3 + nenv) * 8);
#ifdef DACELO_TRACE_CTOR
    if (code == (void *)(uintptr_t)&dc_ctor_step) {
        fprintf(stderr, "  [mk_closure] nenv=%llu env=[%#llx %#llx %#llx]\n",
                (unsigned long long)nenv,
                nenv > 0 ? (unsigned long long)env[0] : 0,
                nenv > 1 ? (unsigned long long)env[1] : 0,
                nenv > 2 ? (unsigned long long)env[2] : 0);
    }
#endif
    blk[0] = HDR(T_CLOSURE, 3 + nenv);
    blk[1] = (uint64_t)(uintptr_t)code;
    blk[2] = nenv;
    memcpy(&blk[3], env, nenv * 8);
    return (value)blk;
}

// partial constructor closure: env = [ctor_id, count, arg0..]
value dc_ctor_step(value clo, value arg) {
    uint64_t *env = &((uint64_t *)clo)[3];
    uint64_t cid = env[0];
    uint64_t count = env[1];
    uint64_t arity = dc_arity_table[cid];
#ifdef DACELO_TRACE_CTOR
    fprintf(stderr, "[ctor-step] clo=%p cid=%llu count=%llu arity=%llu arg=%#llx env=[%#llx %#llx]\n",
            (void*)clo, (unsigned long long)cid, (unsigned long long)count,
            (unsigned long long)arity, (unsigned long long)arg,
            (unsigned long long)env[2], (unsigned long long)env[3]);
#endif
    if (count + 1 == arity) {
        value *fields = (value *)malloc((count + 1) * sizeof(value));
        if (!fields) { fprintf(stderr, "dacelo: oom\n"); exit(1); }
        for (uint64_t i = 0; i < count; i++) fields[i] = env[2 + i];
        fields[count] = arg;
        value r = make_adt(cid, count + 1, fields);
        free(fields);
        return r;
    }
    value *newenv = (value *)malloc((2 + count + 1) * sizeof(value));
    newenv[0] = cid;
    newenv[1] = count + 1;
    for (uint64_t i = 0; i < count; i++) newenv[2 + i] = env[2 + i];
    newenv[2 + count] = arg;
    value r = mk_closure((void *)(uintptr_t)&dc_ctor_step, count + 3, newenv);
    free(newenv);
    return r;
}

value dc_mk_partial_ctor(uint64_t cid) {
    value env[2] = { (value)cid, 0 };
    return mk_closure((void *)(uintptr_t)&dc_ctor_step, 2, env);
}

// ------------------------------------------------------------------
// builtin functions (index-shared with the compiler)
// ------------------------------------------------------------------

enum {
    BI_PRINT_INT = 0, BI_PRINT_STRING, BI_INT_TO_STRING, BI_BOOL_TO_STRING,
    BI_STRING_LENGTH, BI_STR_CONCAT, BI_READ_FILE, BI_WRITE_FILE, BI_EXIT,
    BI_CHR, BI_ORD, BI_STRING_GET, BI_SUBSTRING, BI_STRING_TO_INT,
    BI_ERROR, BI_SHOW, BI_ARGV, BI_SYSTEM, BI_COUNT
};

void dc_fatal(const char *msg) {
    fprintf(stderr, "dacelo: %s\n", msg);
    exit(1);
}

value dc_bi_print_int(value n) {
    if (!IS_INT(n)) dc_fatal("print_int: not an Int");
    printf("%lld", (long long)INT_OF(n));
    return (value)dc_unit_block;
}

value dc_bi_print_string(value s) {
    fwrite(STR_OF(s), 1, STRLEN_OF(s), stdout);
    return (value)dc_unit_block;
}

value dc_bi_int_to_string(value n) {
    char buf[32];
    snprintf(buf, sizeof buf, "%lld", (long long)INT_OF(n));
    return make_string(buf, strlen(buf));
}

value dc_bi_bool_to_string(value b) {
    const char *s = (b == BOOL_TRUE) ? "true" : "false";
    return make_string(s, strlen(s));
}

value dc_bi_string_length(value s) { return MK_INT((int64_t)STRLEN_OF(s)); }

value dc_bi_str_concat(value a, value b) {
    uint64_t la = STRLEN_OF(a), lb = STRLEN_OF(b);
    uint64_t n = la + lb;
    // spare byte so the result is always NUL-terminated (matches make_string)
    uint64_t words = 2 + (n + 8) / 8;
    uint64_t *blk = (uint64_t *)dacelo_alloc(words * 8);
    blk[0] = HDR(T_STRING, words);
    blk[1] = n;
    memcpy(&blk[2], STR_OF(a), la);
    memcpy((char *)&blk[2] + la, STR_OF(b), lb);
    memset((char *)&blk[2] + n, 0, words * 8 - 16 - n);
    return (value)blk;
}

value dc_bi_read_file(value path) {
    FILE *f = fopen(STR_OF(path), "rb");
    if (!f) { fprintf(stderr, "dacelo: cannot open %s\n", STR_OF(path)); exit(1); }
    fseek(f, 0, SEEK_END);
    long sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *buf = (char *)malloc(sz + 1);
    size_t rd = fread(buf, 1, sz, f);
    fclose(f);
    value r = make_string(buf, rd);
    free(buf);
    return r;
}

value dc_bi_write_file(value path, value data) {
    FILE *f = fopen(STR_OF(path), "wb");
    if (!f) { fprintf(stderr, "dacelo: cannot write %s\n", STR_OF(path)); exit(1); }
    fwrite(STR_OF(data), 1, STRLEN_OF(data), f);
    fclose(f);
    return (value)dc_unit_block;
}

value dc_bi_exit(value code) {
    exit((int)INT_OF(code));
}

value dc_bi_chr(value n) {
    int64_t c = INT_OF(n);
    if (c < 0 || c > 0x10FFFF) dc_fatal("chr: invalid code");
    char buf[8];
    int len = 0;
    if (c < 0x80) { buf[len++] = (char)c; }
    else if (c < 0x800) {
        buf[len++] = (char)(0xC0 | (c >> 6));
        buf[len++] = (char)(0x80 | (c & 0x3F));
    } else if (c < 0x10000) {
        buf[len++] = (char)(0xE0 | (c >> 12));
        buf[len++] = (char)(0x80 | ((c >> 6) & 0x3F));
        buf[len++] = (char)(0x80 | (c & 0x3F));
    } else {
        buf[len++] = (char)(0xF0 | (c >> 18));
        buf[len++] = (char)(0x80 | ((c >> 12) & 0x3F));
        buf[len++] = (char)(0x80 | ((c >> 6) & 0x3F));
        buf[len++] = (char)(0x80 | (c & 0x3F));
    }
    return make_string(buf, len);
}

value dc_bi_ord(value s) {
    if (STRLEN_OF(s) == 0) return MK_INT(-1);
    return MK_INT((unsigned char)STR_OF(s)[0]);
}

value dc_bi_string_get(value s, value i) {
    int64_t idx = INT_OF(i);
    if (idx < 0 || (uint64_t)idx >= STRLEN_OF(s)) return MK_INT(-1);
    return MK_INT((unsigned char)STR_OF(s)[idx]);
}

value dc_bi_substring(value s, value start, value len) {
    int64_t st = INT_OF(start), ln = INT_OF(len);
    uint64_t slen = STRLEN_OF(s);
    if (st < 0 || ln < 0 || (uint64_t)st > slen || st + ln > (int64_t)slen)
        dc_fatal("substring: out of bounds");
    return make_string(STR_OF(s) + st, ln);
}

value dc_bi_string_to_int(value s) {
    const char *p = STR_OF(s);
    while (*p == ' ' || *p == '\t' || *p == '\n') p++;
    int neg = 0;
    if (*p == '-') { neg = 1; p++; }
    else if (*p == '+') p++;
    if (*p < '0' || *p > '9') dc_fatal("string_to_int: invalid number");
    long long v = 0;
    while (*p >= '0' && *p <= '9') { v = v * 10 + (*p - '0'); p++; }
    return MK_INT(neg ? -v : v);
}

// structural equality
value dc_val_eq(value a, value b);

static value eq_deep(value a, value b) {
    if (a == b) return BOOL_TRUE;
    uint64_t *pa = (uint64_t *)a, *pb = (uint64_t *)b;
    uint64_t ha = *pa, hb = *pb;
    if (TAG_OF(ha) != TAG_OF(hb)) return BOOL_FALSE;
    switch (TAG_OF(ha)) {
        case T_STRING:
            if (STRLEN_OF(a) != STRLEN_OF(b)) return BOOL_FALSE;
            return memcmp(STR_OF(a), STR_OF(b), STRLEN_OF(a)) == 0 ? BOOL_TRUE : BOOL_FALSE;
        case T_TUPLE: {
            uint64_t n = SIZE_OF(ha);
            for (uint64_t i = 0; i + 1 < n; i++)
                if (dc_val_eq(pa[1 + i], pb[1 + i]) != BOOL_TRUE) return BOOL_FALSE;
            return BOOL_TRUE;
        }
        case T_ADT: {
            if (pa[1] != pb[1]) return BOOL_FALSE;
            uint64_t n = SIZE_OF(ha);
            for (uint64_t i = 0; i + 2 < n; i++)
                if (dc_val_eq(pa[2 + i], pb[2 + i]) != BOOL_TRUE) return BOOL_FALSE;
            return BOOL_TRUE;
        }
        case T_CLOSURE:
            dc_fatal("cannot compare functions");
        default:
            return a == b ? BOOL_TRUE : BOOL_FALSE;
    }
}

value dc_val_eq(value a, value b) {
    if (getenv("DACELO_TRACE_EQ")) {
        fprintf(stderr, "[eq a=%#llx b=%#llx]\n", (unsigned long long)a, (unsigned long long)b);
    }
    if (IS_INT(a) && IS_INT(b)) return a == b ? BOOL_TRUE : BOOL_FALSE;
    if (IS_BOOL(a) && IS_BOOL(b)) return a == b ? BOOL_TRUE : BOOL_FALSE;
    if ((a & 3) || (b & 3)) return BOOL_FALSE;
    return eq_deep(a, b);
}

// show : pretty printer
extern const char *dc_ctor_names[];           // compiler-provided
static void show_into(value v, FILE *out);

static void show_into(value v, FILE *out) {
    if (IS_INT(v)) { fprintf(out, "%lld", (long long)INT_OF(v)); return; }
    if (IS_BOOL(v)) { fputs(v == BOOL_TRUE ? "true" : "false", out); return; }
    if (v & 3) { fprintf(out, "?"); return; }
    uint64_t *p = (uint64_t *)v;
    uint64_t vtag = TAG_OF(*p);
    uint64_t vsize = SIZE_OF(*p);
    if (!chunk_of(p) &&
        !(vtag >= T_STRING && vtag <= T_UNIT && vsize > 0 && vsize < (1 << 20))) {
        // not a heap block and not a plausible static object
        fprintf(out, "?");
        return;
    }
    switch (TAG_OF(*p)) {
        case T_STRING:
            fwrite(STR_OF(v), 1, STRLEN_OF(v), out);
            break;
        case T_UNIT:
            fputs("()", out);
            break;
        case T_TUPLE: {
            uint64_t n = SIZE_OF(*p);
            fputc('(', out);
            for (uint64_t i = 0; i + 1 < n; i++) {
                if (i) fputc(',', out);
                show_into(p[1 + i], out);
            }
            fputc(')', out);
            break;
        }
        case T_ADT: {
            uint64_t cid = p[1];
            const char *nm = dc_ctor_names[cid];
            if (strcmp(nm, "Nil") == 0) { fputs("[]", out); break; }
            if (strcmp(nm, "Cons") == 0) {
                // proper-list spine walk.
                // NOTE: nullary ctor instances (Nil) are STATIC __DATA blocks
                // emitted by the compiler, so validity cannot rely on
                // chunk_of(); a mapped, well-formed header is enough here.
                #define ADTLIKE(val) (((val) & 3) == 0 && \
                        TAG_OF(((uint64_t *)(val))[0]) == T_ADT && \
                        ((uint64_t *)(val))[1] < dc_ctors_registered)
                value cur = v;
                int first = 1;
                fputc('[', out);
                for (;;) {
                    uint64_t *cp = (uint64_t *)cur;
                    if (!ADTLIKE(cur)) break;
                    if (strcmp(dc_ctor_names[cp[1]], "Nil") == 0) break;
                    if (strcmp(dc_ctor_names[cp[1]], "Cons") != 0) break;
                    if (!first) fputc(',', out);
                    show_into(cp[2], out);
                    first = 0;
                    cur = cp[3];
                    if (!ADTLIKE(cur)) { fputc(',', out); show_into(cur, out); break; }
                    if (strcmp(dc_ctor_names[((uint64_t *)cur)[1]], "Nil") == 0) break;
                    if (strcmp(dc_ctor_names[((uint64_t *)cur)[1]], "Cons") == 0) continue;
                    fputc(',', out);
                    show_into(cur, out);
                    break;
                }
                fputc(']', out);
                break;
            }
            uint64_t n = SIZE_OF(*p);
            fprintf(out, "(%s", nm);
            for (uint64_t i = 0; i + 2 < n; i++) {
                fputc(' ', out);
                show_into(p[2 + i], out);
            }
            fputc(')', out);
            break;
        }
        case T_CLOSURE:
            fputs("<fun>", out);
            break;
        default:
            fputs("?", out);
    }
}

value dc_show(value v) {
    char *buf = NULL;
    size_t cap = 0;
    FILE *f = open_memstream(&buf, &cap);
    if (!f) dc_fatal("oom");
    show_into(v, f);
    fclose(f);
    value r = make_string(buf, cap);
    free(buf);
    return r;
}

value dc_argv(value i) {
    extern char **g_user_argv;
    extern int g_user_argc;
    int64_t idx = INT_OF(i);
    // argv 0 is the script name (stored by main), user args follow
    if (idx < 0 || idx >= g_user_argc) return make_string("", 0);
    return make_string(g_user_argv[idx], strlen(g_user_argv[idx]));
}

// builtin dispatcher used by partial applications
value dc_bi_by_index(uint64_t idx, value *args) {
    switch (idx) {
        case BI_PRINT_INT:       return dc_bi_print_int(args[0]);
        case BI_PRINT_STRING:    return dc_bi_print_string(args[0]);
        case BI_INT_TO_STRING:   return dc_bi_int_to_string(args[0]);
        case BI_BOOL_TO_STRING:  return dc_bi_bool_to_string(args[0]);
        case BI_STRING_LENGTH:   return dc_bi_string_length(args[0]);
        case BI_STR_CONCAT:      return dc_bi_str_concat(args[0], args[1]);
        case BI_READ_FILE:       return dc_bi_read_file(args[0]);
        case BI_WRITE_FILE:      return dc_bi_write_file(args[0], args[1]);
        case BI_EXIT:            return dc_bi_exit(args[0]);
        case BI_CHR:             return dc_bi_chr(args[0]);
        case BI_ORD:             return dc_bi_ord(args[0]);
        case BI_STRING_GET:      return dc_bi_string_get(args[0], args[1]);
        case BI_SUBSTRING:       return dc_bi_substring(args[0], args[1], args[2]);
        case BI_STRING_TO_INT:   return dc_bi_string_to_int(args[0]);
        case BI_ERROR:           dc_fatal(STR_OF(args[0]));
        case BI_SHOW:            return dc_show(args[0]);
        case BI_ARGV:            return dc_argv(args[0]);
        case BI_SYSTEM:          return MK_INT(system(STR_OF(args[0])));
    }
    dc_fatal("bad builtin index");
    return 0;
}

static uint64_t bi_arity(uint64_t idx) {
    switch (idx) {
        case BI_STR_CONCAT: case BI_WRITE_FILE: case BI_STRING_GET: return 2;
        case BI_SUBSTRING: return 3;
        default: return 1;
    }
}

// partial builtin closure: env = [index, count, args...]
value dc_builtin_step(value clo, value arg) {
    uint64_t *env = &((uint64_t *)clo)[3];
    uint64_t idx = env[0];
    uint64_t count = env[1];
    uint64_t arity = bi_arity(idx);
    if (count + 1 == arity) {
        value *args = (value *)malloc((count + 1) * sizeof(value));
        for (uint64_t i = 0; i < count; i++) args[i] = env[2 + i];
        args[count] = arg;
        value r = dc_bi_by_index(idx, args);
        free(args);
        return r;
    }
    value *newenv = (value *)malloc((2 + count + 1) * sizeof(value));
    newenv[0] = idx;
    newenv[1] = count + 1;
    for (uint64_t i = 0; i < count; i++) newenv[2 + i] = env[2 + i];
    newenv[2 + count] = arg;
    value r = mk_closure((void *)(uintptr_t)&dc_builtin_step, count + 3, newenv);
    free(newenv);
    return r;
}

value dc_mk_partial_builtin(uint64_t idx) {
    value env[2] = { (value)idx, 0 };
    return mk_closure((void *)(uintptr_t)&dc_builtin_step, 2, env);
}

// division/modulo helper: validates boxed int, returns unboxed value
int64_t dc_div_check(value b) {
    if ((b & 3) != 1) dc_fatal("arithmetic: operand is not an Int");
    int64_t v = INT_OF(b);
    if (v == 0) dc_fatal("division by zero");
    return v;
}

// ------------------------------------------------------------------
// match failure
// ------------------------------------------------------------------

void dc_match_fail(value v) {
    fprintf(stderr, "dacelo: non-exhaustive pattern match on ");
    value m = dc_show(v);
    fwrite(STR_OF(m), 1, STRLEN_OF(m), stderr);
    fputc('\n', stderr);
    exit(1);
}

// ------------------------------------------------------------------
// entry point
// ------------------------------------------------------------------

char **g_user_argv = NULL;
int g_user_argc = 0;
static char *script_name_storage[1];

extern void dc_init(void);
extern void dc_user_main(void);

static void *program_thread(void *arg) {
    (void)arg;
    // conservative stack scan root for this thread
    char marker;
    gc_stack_bottom = (uint64_t *)&marker;
    dc_init();
    dc_user_main();
    fflush(stdout);
    return NULL;
}

int main(int argc, char **argv) {
    // argv 0 = program name; user args start at 1 (matches interpreter view)
    g_user_argv = argv;
    g_user_argc = argc;

    // run the program on a big-stack thread: generated functions use
    // fixed 4 KB frames and dacelo recursion can be deep
    pthread_attr_t attr;
    pthread_attr_init(&attr);
    // large VIRTUAL reservation; pages commit lazily
    if (pthread_attr_setstacksize(&attr, 8ull << 30) != 0) {
        // fall back to whatever the system allows
        pthread_attr_setstacksize(&attr, 512ull << 20);
    }
    pthread_t t;
    if (pthread_create(&t, &attr, program_thread, NULL) != 0) {
        fprintf(stderr, "dacelo: cannot spawn program thread\n");
        return 1;
    }
    pthread_join(t, NULL);
    return 0;
}
