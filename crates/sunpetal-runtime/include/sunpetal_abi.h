// Sunpetal: the ABI between recompiled C and the Rust runtime (docs/ABI.md).
//
// Recompiled functions have the signature `void f(Cpu* c)`. They touch guest
// state only through this header: registers and flags in `Cpu`, memory through
// ld*/st* (which call mem_read*/mem_write* of the runtime), and a few
// out-of-line services (calls through registers, safe points, modes, CP15,
// SWI). Semantics are those of tools/ndskit/armemu.py, instruction for
// instruction; the self-test (crate tests) holds them together.
//
// The layout of `Cpu` and of the module tables is repr(C) on the Rust side
// (src/abi.rs): change both together.
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#define ARM_UNLIKELY(x) __builtin_expect(!!(x), 0)

// Banks of r13/r14 and the SPSR: user/system, FIQ, IRQ, supervisor, abort, undefined.
enum { ARM_BANK_USR, ARM_BANK_FIQ, ARM_BANK_IRQ, ARM_BANK_SVC, ARM_BANK_ABT, ARM_BANK_UND, ARM_NBANKS };

// The memories the CPUs address directly, for the inline fast path of ld*/st*
// (Rust owns them; the pointers never change while the machine lives).
typedef struct MemMap {
    uint8_t* main;                      // 4 MiB at 0x02000000, repeated to 0x02FFFFFF
    uint8_t* itcm;                      // ARM9: 32 KiB, repeated from 0 to 0x01FFFFFF
    uint8_t* dtcm;                      // ARM9: 16 KiB at dtcm_base, over everything else
    uint8_t* wram7;                     // ARM7: 64 KiB at 0x03800000, repeated to 0x03FFFFFF
    uint8_t* swram;                     // shared WRAM, 32 KiB: the ARM7's over 0x03000000-0x037FFFFF
    uint32_t dtcm_base;
    uint32_t dtcm_size;                 // 0x4000 while CP15 enables the DTCM, else 0
    // The overlays' area of each CPU ([0] the ARM9's): a store there goes the slow way,
    // which makes the runtime check the overlay functions over it again.
    uint32_t ovl_lo[2], ovl_len[2];
} MemMap;

typedef struct Cpu {
    uint32_t r[16];                     // r[15] unused: recompiled code knows its pc
    uint32_t n, z, c, v, q;             // the CPSR's flags, one field each
    uint32_t i, f, mode;                // its control bits (T is the function's own)
    uint32_t pc;                        // where the last return or jump went (bit 0: Thumb)
    int32_t  budget;                    // safe points left before arm_poll
    int32_t  cpu;                       // 9 or 7
    uint32_t bank[ARM_NBANKS][2];       // r13, r14 of the banks not in use
    uint32_t fiq_other[5];              // r8-r12 of the other side of FIQ
    uint32_t spsr[ARM_NBANKS];
    uint32_t irq;                       // an interrupt is pending (IME, IE & IF): the runtime keeps it
    const MemMap* mm;                   // set by the runtime when it makes the context
} Cpu;

typedef void (*ArmFunc)(Cpu*);

// A program's recompiled code: its segments as they lie in memory where they
// run (to recognise them), and its entries sorted by key (the address, bit 0
// set for Thumb).
typedef struct ArmFuncEntry { uint32_t key; ArmFunc fn; } ArmFuncEntry;
enum { ARM_SEG_PROGRAM = 0, ARM_SEG_COPIED = 1, ARM_SEG_OVERLAY = 2, ARM_SEG_LOADED = 3 };
typedef struct ArmSegment { uint32_t base, size, crc, kind; } ArmSegment;
typedef struct ArmRange { uint32_t base, size; } ArmRange;
typedef struct ArmModule {
    const char* name;
    int32_t cpu;
    const ArmSegment* segs;
    uint32_t nsegs;
    const ArmFuncEntry* funcs;
    uint32_t nfuncs;
    const uint32_t* landings;           // return addresses of calls to a context save (sorted keys)
    uint32_t nlandings;
    // An overlay's module (overlay >= 0): funcs[i] runs recompiled while the
    // bytes of extents[i] have the crc32 crcs[i].
    int32_t overlay;
    const ArmRange* extents;
    const uint32_t* crcs;
} ArmModule;

// Every module of the build (generated modules.c).
extern const ArmModule* const g_arm_modules[];
extern const int g_arm_nmodules;

// ---- memory (the runtime's bus) -----------------------------------------------------
// The slow path of ld*/st*, for what mem_fast does not hold. Aligned accesses, little-endian;
// the callers align (words to 4, halfwords to 2).
uint32_t mem_read8(Cpu* c, uint32_t a);
uint32_t mem_read16(Cpu* c, uint32_t a);
uint32_t mem_read32(Cpu* c, uint32_t a);
void     mem_write8(Cpu* c, uint32_t a, uint32_t v);
void     mem_write16(Cpu* c, uint32_t a, uint32_t v);
void     mem_write32(Cpu* c, uint32_t a, uint32_t v);

static inline uint32_t arm_ror(uint32_t v, uint32_t n) {
    n &= 31;
    return n ? v >> n | v << (32 - n) : v;
}

// What generated code calls. NDS_CPU (9 or 7) is defined by the module's
// header; without it (modules.c) the CPU is read from the context.
// ldr32 is a word load at any address (the aligned word, rotated); ldrh and
// ldrsh at an odd address read the aligned halfword on the ARM9, and on the
// ARM7 rotate it (ldrh) or read the signed byte (ldrsh).
#ifdef NDS_CPU
#define ARM_IS9(c) (NDS_CPU == 9)
#else
#define ARM_IS9(c) ((c)->cpu == 9)
#endif
// The host memory behind an address of this CPU, or NULL (the hardware: the slow path).
static inline uint8_t* mem_fast(const Cpu* c, uint32_t a) {
    const MemMap* m = c->mm;
    if (ARM_IS9(c)) {
        if (a - m->dtcm_base < m->dtcm_size) return m->dtcm + (a - m->dtcm_base);
        if (a < 0x02000000u) return m->itcm + (a & 0x7FFFu);
    }
    if (a >> 24 == 0x02) return m->main + (a & 0x3FFFFFu);
    if (!ARM_IS9(c)) {
        if (a >> 23 == 7) return m->wram7 + (a & 0xFFFFu);
        if (a >> 23 == 6) return m->swram + (a & 0x7FFFu);
    }
    return NULL;
}
static inline uint8_t* mem_fast_w(const Cpu* c, uint32_t a) {
    const int i = ARM_IS9(c) ? 0 : 1;
    if (ARM_UNLIKELY(a - c->mm->ovl_lo[i] < c->mm->ovl_len[i])) return NULL;
    return mem_fast(c, a);
}
static inline uint32_t ld8(Cpu* c, uint32_t a) {
    uint8_t* p = mem_fast(c, a);
    return p ? *p : mem_read8(c, a);
}
static inline uint32_t ld16(Cpu* c, uint32_t a) {
    uint8_t* p = mem_fast(c, a);
    if (!p) return mem_read16(c, a);
    uint16_t v;
    memcpy(&v, p, 2);
    return v;
}
static inline uint32_t ld32(Cpu* c, uint32_t a) {
    uint8_t* p = mem_fast(c, a);
    if (!p) return mem_read32(c, a);
    uint32_t v;
    memcpy(&v, p, 4);
    return v;
}
static inline void st8(Cpu* c, uint32_t a, uint32_t v) {
    uint8_t* p = mem_fast_w(c, a);
    if (p) *p = (uint8_t)v; else mem_write8(c, a, v);
}
static inline void st16(Cpu* c, uint32_t a, uint32_t v) {
    uint8_t* p = mem_fast_w(c, a);
    if (!p) { mem_write16(c, a, v); return; }
    uint16_t w = (uint16_t)v;
    memcpy(p, &w, 2);
}
static inline void st32(Cpu* c, uint32_t a, uint32_t v) {
    uint8_t* p = mem_fast_w(c, a);
    if (p) memcpy(p, &v, 4); else mem_write32(c, a, v);
}
static inline uint32_t ldr32(Cpu* c, uint32_t a) { return arm_ror(mem_read32(c, a & ~3u), (a & 3u) * 8); }
static inline uint32_t ldrh(Cpu* c, uint32_t a) {
    if (ARM_IS9(c) || !(a & 1)) return mem_read16(c, a & ~1u);
    return arm_ror(mem_read16(c, a & ~1u), 8);
}
static inline uint32_t ldrsh(Cpu* c, uint32_t a) {
    if (ARM_IS9(c) || !(a & 1)) return (uint32_t)(int16_t)mem_read16(c, a & ~1u);
    return (uint32_t)(int8_t)mem_read8(c, a);
}

// ---- the shifter ---------------------------------------------------------------------
// By a register: the amount is the bottom byte of Rs (0-255).
static inline uint32_t arm_lsl(uint32_t v, uint32_t n) { return n < 32 ? v << n : 0; }
static inline uint32_t arm_lsr(uint32_t v, uint32_t n) { return n < 32 ? v >> n : 0; }
static inline uint32_t arm_asr(uint32_t v, uint32_t n) { return (uint32_t)((int32_t)v >> (n < 32 ? n : 31)); }
// ... and the carry out, into *cy (left alone for an amount of 0)
static inline uint32_t arm_lsl_c(uint32_t v, uint32_t n, uint32_t* cy) {
    if (n == 0) return v;
    if (n < 32) { *cy = v >> (32 - n) & 1; return v << n; }
    *cy = n == 32 ? v & 1 : 0;
    return 0;
}
static inline uint32_t arm_lsr_c(uint32_t v, uint32_t n, uint32_t* cy) {
    if (n == 0) return v;
    if (n < 32) { *cy = v >> (n - 1) & 1; return v >> n; }
    *cy = n == 32 ? v >> 31 : 0;
    return 0;
}
static inline uint32_t arm_asr_c(uint32_t v, uint32_t n, uint32_t* cy) {
    if (n == 0) return v;
    if (n < 32) { *cy = v >> (n - 1) & 1; return (uint32_t)((int32_t)v >> n); }
    *cy = v >> 31;
    return (uint32_t)((int32_t)v >> 31);
}
static inline uint32_t arm_ror_c(uint32_t v, uint32_t n, uint32_t* cy) {
    if (n == 0) return v;
    v = arm_ror(v, n);
    *cy = v >> 31;
    return v;
}

// ---- saturating arithmetic (ARMv5TE) ---------------------------------------------------
static inline uint32_t arm_sat(Cpu* c, int64_t x) {
    if (x > INT32_MAX) { c->q = 1; return (uint32_t)INT32_MAX; }
    if (x < INT32_MIN) { c->q = 1; return (uint32_t)INT32_MIN; }
    return (uint32_t)x;
}
static inline uint32_t arm_qadd(Cpu* c, uint32_t m, uint32_t n) {
    return arm_sat(c, (int64_t)(int32_t)m + (int32_t)n);
}
static inline uint32_t arm_qsub(Cpu* c, uint32_t m, uint32_t n) {
    return arm_sat(c, (int64_t)(int32_t)m - (int32_t)n);
}
static inline uint32_t arm_qdadd(Cpu* c, uint32_t m, uint32_t n) {
    return arm_sat(c, (int64_t)(int32_t)m + (int32_t)arm_sat(c, 2 * (int64_t)(int32_t)n));
}
static inline uint32_t arm_qdsub(Cpu* c, uint32_t m, uint32_t n) {
    return arm_sat(c, (int64_t)(int32_t)m - (int32_t)arm_sat(c, 2 * (int64_t)(int32_t)n));
}

// ---- the status registers --------------------------------------------------------------
static inline uint32_t arm_get_cpsr(const Cpu* c) {
    return c->n << 31 | c->z << 30 | c->c << 29 | c->v << 28 | c->q << 27 | c->i << 7 | c->f << 6 | c->mode;
}
void arm_set_mode(Cpu* c, uint32_t mode);              // switches the banked registers
// msr cpsr_<fields>: mask c=1, x=2, s=4, f=8; in user mode only the flags change.
static inline void arm_set_cpsr(Cpu* c, uint32_t v, uint32_t mask) {
    if (mask & 8) {
        c->n = v >> 31; c->z = v >> 30 & 1; c->c = v >> 29 & 1; c->v = v >> 28 & 1;
        if (c->cpu == 9) c->q = v >> 27 & 1;
    }
    if (mask & 1 && c->mode != 0x10) {
        c->i = v >> 7 & 1; c->f = v >> 6 & 1;
        arm_set_mode(c, (v & 0x1F) | 0x10);
        if (c->irq && !c->i) c->budget = 0;     // unmasked with one pending: taken at the next safe point
    }
}
static inline void arm_set_nzcv(Cpu* c, uint32_t v) {
    c->n = v >> 31; c->z = v >> 30 & 1; c->c = v >> 29 & 1; c->v = v >> 28 & 1;
}
uint32_t arm_get_spsr(const Cpu* c);                   // the CPSR in user and system mode
void     arm_set_spsr(Cpu* c, uint32_t v, uint32_t mask);
// subs pc, lr, #n / ldm ..., {..., pc}^: CPSR = SPSR, then c->pc = target in the restored state
void     arm_restore_cpsr(Cpu* c, uint32_t target);
void     arm_exception_return(Cpu* c, uint32_t target);
uint32_t arm_user_reg(const Cpu* c, int n);            // ldm/stm with ^
void     arm_set_user_reg(Cpu* c, int n, uint32_t v);

// The address a bx/blx (or, on the ARMv5, a load into pc) goes to: bit 0 the mode.
static inline uint32_t arm_iw(uint32_t x) { return x & 1u ? x : x & ~3u; }

// ---- services of the runtime -----------------------------------------------------------
void     arm_call(Cpu* c, uint32_t key);               // a call through a register: an entry, or not
void     arm_jump(Cpu* c, uint32_t key);               // a jump: an entry is called; elsewhere is a return
void     arm_poll(Cpu* c);                             // the budget is spent: time, interrupts
void     arm_swi(Cpu* c, uint32_t num, uint32_t ret);
void     arm_undefined(Cpu* c, uint32_t pc);
uint32_t arm_cp15_read(Cpu* c, int op1, int crn, int crm, int op2);
void     arm_cp15_write(Cpu* c, int op1, int crn, int crm, int op2, uint32_t v);
// A return went elsewhere than to lr. Not a fatal error when c->pc is a
// landing of the active module (a saved context was loaded): then it comes
// back, and the caller returns (C has no exceptions: every function on the
// way returns, the one holding the landing goes on there). Otherwise it does
// not come back.
void     arm_bad_return(Cpu* c, uint32_t expected);
void     arm_context_saved(Cpu* c);                    // a context save returned (its caller's sp)
// Run the guest code at `key` (bit 0: Thumb) from memory as it is now, until
// it returns to lr as it was at the call: code no module holds or the emitter
// could not take.
void     arm_interpret(Cpu* c, uint32_t key);

// A safe point: backward jumps, calls, and an msr that may unmask interrupts.
#define ARM_POLL(c) do { if (ARM_UNLIKELY(--(c)->budget < 0)) arm_poll(c); } while (0)
// At each function's entry: when the runtime traces calls (a debugging aid),
// it is told; otherwise one predictable branch.
extern int g_arm_trace;
void arm_trace_enter(Cpu* c, uint32_t key);
#define ARM_ENTER(c, k) do { if (ARM_UNLIKELY(g_arm_trace)) arm_trace_enter(c, k); } while (0)
// After a call: the callee's return must have come back here. If it did not,
// arm_bad_return decides (see there); coming back means an unwind in progress.
#define ARM_RET(c, a) do { if (ARM_UNLIKELY((c)->pc != (a))) { arm_bad_return(c, a); return; } } while (0)
