// ============================================================
// rt5.c -- dacelo Gen 5 native runtime
//
// memory model : precise reference counting (Perceus style), no tracing GC
// values       : every value is a 64-bit word
//                int   : (n << 2) | 1          (odd)
//                bool  : false=3, true=7       (odd)
//                other : 8-byte aligned pointer to a block (even)
// block        : [header][payload...]
//                header = (rc << 40) | (size_words << 8) | tag
//                rc     : bits 40..59; RC_IMMORTAL (0xFFFFF) marks static data
//                         and saturated counts: dup/drop are no-ops on it
//                size   : bits 8..39, in 8-byte words, header included
//                tag    : bits 0..6
//   1 String   [hdr][len][bytes...]        no children
//   2 Tuple    [hdr][e1..en]               children: words 1..
//   3 ADT      [hdr][ctor_id][f1..fn]      children: words 2..
//   4 Closure  [hdr][code][env_size][env]  children: words 3..
//   5 Unit     static singleton dc_unit_block
//   0x7E Freed on the free list (word 1 = next); touching it is a bug
//
// ownership    : closure call code(clo, arg): clo BORROWED, arg OWNED by
//                the callee.  Builtins borrow every argument and return a
//                fresh owned block, an immediate or an immortal static.
//                dc_gset moves ownership into the global table; dc_gget
//                returns a borrowed reference.
// ============================================================

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>

typedef uint64_t value;

#define RC_SHIFT      40
#define RC_ONE        (1ULL << RC_SHIFT)
#define RC_IMMORTAL   0xFFFFFULL
#define RC_MASK       (RC_IMMORTAL << RC_SHIFT)
#define TAG_OF(h)     ((h) & 0x7F)
#define SIZE_OF(h)    (((h) >> 8) & 0xFFFFFFFFULL)
#define RC_OF(h)      (((h) >> RC_SHIFT) & RC_IMMORTAL)
#define HDR_RC(tag, size, rc) \
    (((uint64_t)(rc) << RC_SHIFT) | ((uint64_t)(size) << 8) | (uint64_t)(tag))
#define HDR(tag, size)        HDR_RC(tag, size, 1)            // fresh owned block
#define HDR_STATIC(tag, size) HDR_RC(tag, size, RC_IMMORTAL)  // static data
#define HDR_FREED(size)       HDR_RC(T_FREED, size, 0)        // on a free list

enum {
    T_STRING  = 1,
    T_TUPLE   = 2,
    T_ADT     = 3,
    T_CLOSURE = 4,
    T_UNIT    = 5,
    T_FREED   = 0x7E,
};

#define MK_INT(n)   ((((uint64_t)(intptr_t)(n)) << 2) | 1)
#define INT_OF(v)   (((int64_t)(v)) >> 2)
#define IS_INT(v)   (((v) & 3) == 1)
#define BOOL_FALSE  ((value)3)
#define BOOL_TRUE   ((value)7)
#define IS_BOOL(v)  (((v) & 3) == 3)
#define IS_IMM(v)   ((v) & 1)                 // every immediate is odd

// static unit singleton; exported so generated code can load it directly
uint64_t dc_unit_block[1] = { HDR_STATIC(T_UNIT, 1) };

__attribute__((noreturn)) void dc_fatal(const char *msg) {
    fprintf(stderr, "dacelo: %s\n", msg);
    exit(1);
}

__attribute__((noreturn)) static void rc_use_after_free(void) {
    fflush(stdout);
    fprintf(stderr, "dacelo: use after free\n");
    abort();
}

__attribute__((noreturn)) static void rc_oom(void) {
    fprintf(stderr, "dacelo: out of memory\n");
    exit(1);
}

// ------------------------------------------------------------------
// heap: bump allocation from 8 MB chunks + segregated free lists
//
// 2..SMALL_MAX words: one exact-size list each.  Bigger blocks: 256 size
// classes, eight per power of two (12.5% granularity), so any block of a
// higher class satisfies a request of a lower one and the in-class search
// can be bounded.  Runtime-made strings round their capacity up to a
// class boundary, which lets the very common "acc = acc ^ piece" loop
// recycle the block it just freed instead of growing the heap
// quadratically.  A small request whose exact list is empty is carved
// from the smallest free big block.  Memory is never returned to the OS
// and neighbouring free blocks are not coalesced.
// ------------------------------------------------------------------

#define CHUNK_WORDS (1024 * 1024)   // 8 MB per chunk
#define SMALL_MAX   64              // exact-size lists for 2..64 words
#define NBIG        256             // 32 powers of two x 8 sub-classes
#define BIG_SCAN    16              // in-class blocks examined per big alloc

typedef struct FreeBlock {
    uint64_t hdr;                   // HDR_FREED(size)
    struct FreeBlock *next;
} FreeBlock;

static uint64_t *bump_ptr = NULL;
static uint64_t *bump_end = NULL;
static FreeBlock *free_small[SMALL_MAX + 1];
static FreeBlock *free_big[NBIG];
static uint64_t big_nonempty[NBIG / 64];   // bit c set <=> free_big[c] != NULL

// statistics (DACELO_RC_STATS) and the live count (DACELO_RC_CHECK)
static uint64_t n_allocs = 0, n_frees = 0, n_reuses = 0, n_live = 0, n_peak = 0;

// size class of a big block: (floor(log2 size), next three mantissa bits)
static inline unsigned big_class(uint64_t words) {
    unsigned k = 63 - (unsigned)__builtin_clzll(words);
    return (k << 3) | (unsigned)((words >> (k - 3)) & 7);
}

// round a big size up to the next class boundary (at most 12.5% slack)
static inline uint64_t round_big(uint64_t words) {
    uint64_t step = 1ULL << ((63 - __builtin_clzll(words)) - 3);
    return (words + step - 1) & ~(step - 1);
}

static inline void big_set_bit(unsigned c, int on) {
    if (on) big_nonempty[c >> 6] |=  1ULL << (c & 63);
    else    big_nonempty[c >> 6] &= ~(1ULL << (c & 63));
}

// lowest non-empty big class >= from, or -1
static int first_nonempty_class(unsigned from) {
    for (unsigned w = from >> 6; w < NBIG / 64; w++) {
        uint64_t bits = big_nonempty[w];
        if (w == (from >> 6)) bits &= ~0ULL << (from & 63);
        if (bits) return (int)(w * 64 + (unsigned)__builtin_ctzll(bits));
    }
    return -1;
}

// put a block of `size` words (size >= 2) on the free list for its size
static void push_free(uint64_t *p, uint64_t size) {
    FreeBlock *fb = (FreeBlock *)p;
    FreeBlock **head;
    if (size <= SMALL_MAX) head = &free_small[size];
    else { unsigned c = big_class(size); head = &free_big[c]; big_set_bit(c, 1); }
    fb->hdr  = HDR_FREED(size);
    fb->next = *head;
    *head = fb;
}

static uint64_t *pop_big(unsigned c) {          // free_big[c] must be non-empty
    FreeBlock *fb = free_big[c];
    free_big[c] = fb->next;
    if (!fb->next) big_set_bit(c, 0);
    return (uint64_t *)fb;
}

// take `words` from the front of free block p; the remainder goes back on
// a list.  A 1-word remainder cannot hold a free-block header and is
// abandoned (8 bytes, rare).
static uint64_t *carve(uint64_t *p, uint64_t words) {
    uint64_t rem = SIZE_OF(p[0]) - words;
    if (rem >= 2) push_free(p + words, rem);
    return p;
}

static uint64_t *big_alloc(uint64_t words) {
    unsigned c = big_class(words);
    FreeBlock **prev = &free_big[c];
    for (int n = 0; *prev && n < BIG_SCAN; n++) {   // same class: bounded first-fit
        FreeBlock *fb = *prev;
        if (SIZE_OF(fb->hdr) >= words) {
            *prev = fb->next;
            if (!free_big[c]) big_set_bit(c, 0);
            return carve((uint64_t *)fb, words);
        }
        prev = &fb->next;
    }
    int k = first_nonempty_class(c + 1);            // any higher-class block fits
    return k >= 0 ? carve(pop_big((unsigned)k), words) : NULL;
}

static uint64_t *bump_alloc(uint64_t words) {
    uint64_t avail = bump_ptr ? (uint64_t)(bump_end - bump_ptr) : 0;
    if (avail < words) {
        if (avail >= 2) push_free(bump_ptr, avail);  // recycle the chunk tail
        uint64_t cw = words > CHUNK_WORDS ? words : CHUNK_WORDS;
        uint64_t *mem = (uint64_t *)malloc(cw * sizeof(uint64_t));
        if (!mem) rc_oom();
        bump_ptr = mem;
        bump_end = mem + cw;
    }
    uint64_t *p = bump_ptr;
    bump_ptr += words;
    return p;
}

// Allocate block memory of at least nbytes (>= 2 words).  The caller writes
// the header ((1<<40)|(words<<8)|tag) and every payload word.
value dc_alloc_rc(uint64_t nbytes) {
    uint64_t words = (nbytes + 7) >> 3;
    if (words < 2) words = 2;
    if (words > 0xFFFFFFFFULL) dc_fatal("allocation too large");
    uint64_t *blk;
    if (words <= SMALL_MAX) {
        FreeBlock *fb = free_small[words];
        if (fb) { free_small[words] = fb->next; blk = (uint64_t *)fb; }
        else {
            int k = first_nonempty_class(0);        // nibble the smallest big block
            blk = k >= 0 ? carve(pop_big((unsigned)k), words) : bump_alloc(words);
        }
    } else {
        blk = big_alloc(words);
        if (!blk) blk = bump_alloc(words);
    }
    n_allocs++;
    if (++n_live > n_peak) n_peak = n_live;
    return (value)blk;
}

// compatibility aliases (Gen3 code and the runtime's own helpers)
void *dacelo_alloc(uint64_t nbytes)   { return (void *)dc_alloc_rc(nbytes); }
void *dc_alloc_bytes(uint64_t nbytes) { return (void *)dc_alloc_rc(nbytes); }

// ------------------------------------------------------------------
// reference counting
// ------------------------------------------------------------------

// worklist of blocks whose count reached zero and still need releasing;
// an explicit stack keeps dc_drop iterative on arbitrarily long lists
static uint64_t **wl = NULL;
static size_t wl_sp = 0, wl_cap = 0;

static void wl_push(uint64_t *p) {
    if (wl_sp == wl_cap) {
        wl_cap = wl_cap ? wl_cap * 2 : 4096;
        wl = (uint64_t **)realloc(wl, wl_cap * sizeof(uint64_t *));
        if (!wl) rc_oom();
    }
    wl[wl_sp++] = p;
}

// return a dead block to its free list
static void free_block(uint64_t *blk) {
    push_free(blk, SIZE_OF(*blk));
    n_frees++;
    n_live--;
}

// Give up blk's references to its children (blk itself is not touched).
// Children whose count reaches zero are queued instead of released
// recursively.  A word equal to blk's own address is the non-owning self
// reference of a local `let rec` closure and is skipped.
static void release_children(uint64_t *blk) {
    uint64_t h = *blk;
    uint64_t size = SIZE_OF(h);
    uint64_t first;
    switch (TAG_OF(h)) {
        case T_TUPLE:   first = 1; break;
        case T_ADT:     first = 2; break;          // word 1 is the raw ctor id
        case T_CLOSURE: first = 3; break;          // words 1,2: code, env size
        default:        return;                    // String / Unit: no children
    }
    for (uint64_t i = first; i < size; i++) {
        value w = blk[i];
        if (IS_IMM(w) || w == 0 || w == (value)blk) continue;
        uint64_t *c = (uint64_t *)w;
        uint64_t ch = *c;
        if (TAG_OF(ch) == T_FREED) rc_use_after_free();
        uint64_t rc = RC_OF(ch);
        if (rc == RC_IMMORTAL) continue;
        if (rc > 1) { *c = ch - RC_ONE; continue; }
        if (rc == 0) dc_fatal("refcount underflow");
        wl_push(c);                                // last reference: release
    }
}

static void wl_drain(void) {
    while (wl_sp > 0) {
        uint64_t *b = wl[--wl_sp];
        if (TAG_OF(*b) == T_FREED) rc_use_after_free();  // queued twice: bad count
        release_children(b);
        free_block(b);
    }
}

void dc_dup(value v) {
    if (IS_IMM(v) || v == 0) return;
    uint64_t *p = (uint64_t *)v;
    uint64_t h = *p;
    if (TAG_OF(h) == T_FREED) rc_use_after_free();
    if (RC_OF(h) == RC_IMMORTAL) return;
    *p = h + RC_ONE;                               // 0xFFFFE+1 saturates to immortal
}

void dc_drop(value v) {
    if (IS_IMM(v) || v == 0) return;
    uint64_t *p = (uint64_t *)v;
    uint64_t h = *p;
    if (TAG_OF(h) == T_FREED) rc_use_after_free();
    uint64_t rc = RC_OF(h);
    if (rc == RC_IMMORTAL) return;
    if (rc > 1) { *p = h - RC_ONE; return; }
    if (rc == 0) dc_fatal("refcount underflow");
    release_children(p);
    free_block(p);
    wl_drain();
}

// If v is a uniquely owned heap block, release its children and hand the
// block back for reuse (header keeps size/tag, rc stays 1).  Otherwise
// drop v and return 0.
value dc_drop_reuse(value v) {
    if (IS_IMM(v) || v == 0) return 0;
    uint64_t *p = (uint64_t *)v;
    uint64_t h = *p;
    if (TAG_OF(h) == T_FREED) rc_use_after_free();
    if (RC_OF(h) != 1) { dc_drop(v); return 0; }
    release_children(p);
    wl_drain();
    return v;
}

// Use the reuse token if it has exactly the requested size; otherwise free
// it (its children are already gone) and allocate fresh.
value dc_reuse_or_alloc(value tok, uint64_t nbytes) {
    if (tok && !IS_IMM(tok)) {
        uint64_t *p = (uint64_t *)tok;
        uint64_t h = *p;
        if (TAG_OF(h) == T_FREED) rc_use_after_free();
        uint64_t words = (nbytes + 7) >> 3;
        if (words < 2) words = 2;
        if (SIZE_OF(h) == words) { n_reuses++; return tok; }
        free_block(p);
    }
    return dc_alloc_rc(nbytes);
}

// ------------------------------------------------------------------
// constructors for runtime values (all return fresh blocks with rc = 1)
// ------------------------------------------------------------------

// block size for a string of len bytes: one spare byte is reserved so the
// content is ALWAYS NUL-terminated for C interop (fopen/fprintf), even when
// len is a multiple of 8; big strings round up to a size-class boundary
// (see the heap notes above)
static uint64_t str_words(uint64_t len) {
    uint64_t words = 2 + (len + 8) / 8;
    return words > SMALL_MAX ? round_big(words) : words;
}

static value make_string(const char *bytes, uint64_t len) {
    uint64_t words = str_words(len);
    uint64_t *blk = (uint64_t *)dc_alloc_rc(words * 8);
    blk[0] = HDR(T_STRING, words);
    blk[1] = len;
    memcpy(&blk[2], bytes, len);
    memset((char *)&blk[2] + len, 0, words * 8 - 16 - len);
    return (value)blk;
}

#define STR_OF(v)    ((const char *)&((uint64_t *)(v))[2])
#define STRLEN_OF(v) (((uint64_t *)(v))[1])

// closure layout: [hdr][code][env_size][env...]; env words left to the caller
static uint64_t *alloc_closure(void *code, uint64_t nenv) {
    uint64_t *blk = (uint64_t *)dc_alloc_rc((3 + nenv) * 8);
    blk[0] = HDR(T_CLOSURE, 3 + nenv);
    blk[1] = (uint64_t)(uintptr_t)code;
    blk[2] = nenv;
    return blk;
}

// ------------------------------------------------------------------
// global variable table (owns its entries; dc_gget borrows)
// ------------------------------------------------------------------

uint64_t dc_global_table[1 << 16];
uint64_t dc_global_count = 0;

void dc_gset(uint64_t i, value v) {
    value old = dc_global_table[i];
    dc_global_table[i] = v;
    if (i + 1 > dc_global_count) dc_global_count = i + 1;
    if (old) dc_drop(old);                         // slots are normally set once
}

value dc_gget(uint64_t i) {
    return dc_global_table[i];
}

// ------------------------------------------------------------------
// constructor arity table + partial-application machinery
//
// Trampoline closures keep their bookkeeping words (ctor id / builtin
// index, argument count) as tagged Ints so the precise release never
// mistakes them for pointers.
// ------------------------------------------------------------------

uint64_t dc_arity_table[4096];                // ctor_id -> field count
uint64_t dc_ctors_registered = 0;

void dc_register_ctor(uint64_t id, uint64_t arity) {
    dc_arity_table[id] = arity;
    if (id + 1 > dc_ctors_registered) dc_ctors_registered = id + 1;
}

// partial constructor closure: env = [MK_INT(ctor_id), MK_INT(count), arg0..]
// clo is borrowed, arg is owned; copied env args are dup'd (clo keeps its own)
value dc_ctor_step(value clo, value arg) {
    uint64_t *env = &((uint64_t *)clo)[3];
    uint64_t cid   = (uint64_t)INT_OF(env[0]);
    uint64_t count = (uint64_t)INT_OF(env[1]);
    uint64_t arity = dc_arity_table[cid];
    if (count + 1 == arity) {
        uint64_t *blk = (uint64_t *)dc_alloc_rc((3 + count) * 8);
        blk[0] = HDR(T_ADT, 3 + count);
        blk[1] = cid;
        for (uint64_t i = 0; i < count; i++) { dc_dup(env[2 + i]); blk[2 + i] = env[2 + i]; }
        blk[2 + count] = arg;
        return (value)blk;
    }
    uint64_t *blk = alloc_closure((void *)(uintptr_t)&dc_ctor_step, count + 3);
    blk[3] = MK_INT(cid);
    blk[4] = MK_INT(count + 1);
    for (uint64_t i = 0; i < count; i++) { dc_dup(env[2 + i]); blk[5 + i] = env[2 + i]; }
    blk[5 + count] = arg;
    return (value)blk;
}

value dc_mk_partial_ctor(uint64_t cid) {
    uint64_t *blk = alloc_closure((void *)(uintptr_t)&dc_ctor_step, 2);
    blk[3] = MK_INT(cid);
    blk[4] = MK_INT(0);
    return (value)blk;
}

// ------------------------------------------------------------------
// builtin functions (index-shared with the compiler); arguments borrowed
// ------------------------------------------------------------------

enum {
    BI_PRINT_INT = 0, BI_PRINT_STRING, BI_INT_TO_STRING, BI_BOOL_TO_STRING,
    BI_STRING_LENGTH, BI_STR_CONCAT, BI_READ_FILE, BI_WRITE_FILE, BI_EXIT,
    BI_CHR, BI_ORD, BI_STRING_GET, BI_SUBSTRING, BI_STRING_TO_INT,
    BI_ERROR, BI_SHOW, BI_ARGV, BI_SYSTEM, BI_COUNT
};

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
    uint64_t words = str_words(n);
    uint64_t *blk = (uint64_t *)dc_alloc_rc(words * 8);
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
    if (sz < 0) sz = 0;
    fseek(f, 0, SEEK_SET);
    char *buf = (char *)malloc((size_t)sz + 1);
    if (!buf) rc_oom();
    size_t rd = fread(buf, 1, (size_t)sz, f);
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
    return make_string(buf, (uint64_t)len);
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
    return make_string(STR_OF(s) + st, (uint64_t)ln);
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

value dc_bi_error(value msg) {
    dc_fatal(STR_OF(msg));
}

value dc_bi_system(value cmd) {
    return MK_INT(system(STR_OF(cmd)));
}

// structural equality (borrows both operands)
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
    if (IS_INT(a) && IS_INT(b)) return a == b ? BOOL_TRUE : BOOL_FALSE;
    if (IS_BOOL(a) && IS_BOOL(b)) return a == b ? BOOL_TRUE : BOOL_FALSE;
    if ((a & 3) || (b & 3)) return BOOL_FALSE;
    return eq_deep(a, b);
}

// show : pretty printer (borrows its argument)
extern const char *dc_ctor_names[];           // compiler-provided

static void show_into(value v, FILE *out) {
    if (IS_INT(v)) { fprintf(out, "%lld", (long long)INT_OF(v)); return; }
    if (IS_BOOL(v)) { fputs(v == BOOL_TRUE ? "true" : "false", out); return; }
    if (v & 3) { fputs("?", out); return; }
    uint64_t *p = (uint64_t *)v;
    uint64_t vtag = TAG_OF(*p);
    uint64_t vsize = SIZE_OF(*p);
    if (!(vtag >= T_STRING && vtag <= T_UNIT && vsize > 0)) {
        fputs("?", out);                           // not a well-formed block
        return;
    }
    switch (vtag) {
        case T_STRING:
            fwrite(STR_OF(v), 1, STRLEN_OF(v), out);
            break;
        case T_UNIT:
            fputs("()", out);
            break;
        case T_TUPLE: {
            fputc('(', out);
            for (uint64_t i = 0; i + 1 < vsize; i++) {
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
                // proper-list spine walk; Nil is a static block, so only the
                // header shape is checked (never the allocator)
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
            fprintf(out, "(%s", nm);
            for (uint64_t i = 0; i + 2 < vsize; i++) {
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

value dc_bi_show(value v) { return dc_show(v); }

char **g_user_argv = NULL;
int g_user_argc = 0;

value dc_argv(value i) {
    int64_t idx = INT_OF(i);
    // argv 0 is the script name (stored by main), user args follow
    if (idx < 0 || idx >= g_user_argc) return make_string("", 0);
    return make_string(g_user_argv[idx], strlen(g_user_argv[idx]));
}

value dc_bi_argv(value i) { return dc_argv(i); }

// builtin dispatcher used by partial applications (args borrowed)
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
        case BI_ERROR:           return dc_bi_error(args[0]);
        case BI_SHOW:            return dc_bi_show(args[0]);
        case BI_ARGV:            return dc_bi_argv(args[0]);
        case BI_SYSTEM:          return dc_bi_system(args[0]);
    }
    dc_fatal("bad builtin index");
}

static uint64_t bi_arity(uint64_t idx) {
    switch (idx) {
        case BI_STR_CONCAT: case BI_WRITE_FILE: case BI_STRING_GET: return 2;
        case BI_SUBSTRING: return 3;
        default: return 1;
    }
}

// partial builtin closure: env = [MK_INT(index), MK_INT(count), args...]
// clo is borrowed, arg is owned: the builtin borrows it, then it is dropped
value dc_builtin_step(value clo, value arg) {
    uint64_t *env = &((uint64_t *)clo)[3];
    uint64_t idx   = (uint64_t)INT_OF(env[0]);
    uint64_t count = (uint64_t)INT_OF(env[1]);
    uint64_t arity = bi_arity(idx);
    if (count + 1 == arity) {
        value args[3];
        for (uint64_t i = 0; i < count; i++) args[i] = env[2 + i];
        args[count] = arg;
        value r = dc_bi_by_index(idx, args);
        dc_drop(arg);
        return r;
    }
    uint64_t *blk = alloc_closure((void *)(uintptr_t)&dc_builtin_step, count + 3);
    blk[3] = MK_INT(idx);
    blk[4] = MK_INT(count + 1);
    for (uint64_t i = 0; i < count; i++) { dc_dup(env[2 + i]); blk[5 + i] = env[2 + i]; }
    blk[5 + count] = arg;
    return (value)blk;
}

value dc_mk_partial_builtin(uint64_t idx) {
    uint64_t *blk = alloc_closure((void *)(uintptr_t)&dc_builtin_step, 2);
    blk[3] = MK_INT(idx);
    blk[4] = MK_INT(0);
    return (value)blk;
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
// shutdown diagnostics and entry point
// ------------------------------------------------------------------

// Called by the runtime after dc_user_main.
//   DACELO_RC_STATS=1 : allocation counters to stderr
//   DACELO_RC_CHECK=1 : drop every global, then fail (exit 3) if any block
//                       is still live -- i.e. the generated code leaked
void dc_rt_shutdown(void) {
    if (getenv("DACELO_RC_STATS")) {
        fprintf(stderr, "rc: allocs=%llu frees=%llu reuses=%llu peak_live=%llu\n",
                (unsigned long long)n_allocs, (unsigned long long)n_frees,
                (unsigned long long)n_reuses, (unsigned long long)n_peak);
    }
    if (getenv("DACELO_RC_CHECK")) {
        for (uint64_t i = 0; i < dc_global_count; i++) {
            value v = dc_global_table[i];
            dc_global_table[i] = 0;
            dc_drop(v);
        }
        if (n_live != 0) {
            fprintf(stderr, "dacelo: LEAK %llu blocks\n", (unsigned long long)n_live);
            exit(3);
        }
    }
}

extern void dc_init(void);
extern void dc_user_main(void);

static void *program_thread(void *arg) {
    (void)arg;
    dc_init();
    dc_user_main();
    fflush(stdout);
    dc_rt_shutdown();
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
