    .intel_syntax noprefix
    .text
# pick(u): r = probe(u); switch (r) { case 1: u->speed = 0; report(u, 1); break;
#          case 0: default: u->speed = 15; report(u, 15); }
# with zero kept in ebx, as MSVC /O2 does: sub eax,ebx; je; sub eax,1; jne
    .globl _pick
_pick:
    push ebx
    push esi
    mov esi, dword ptr [esp + 12]
    xor ebx, ebx
    push esi
    call _probe
    add esp, 4
    sub eax, ebx
    je .Ldefault
    sub eax, 1
    jne .Ldefault
    mov dword ptr [esi + 4], ebx
    push 1
    jmp .Lreport
.Ldefault:
    push 15
    mov dword ptr [esi + 4], 15
.Lreport:
    push esi
    call _report
    add esp, 8
    pop esi
    pop ebx
    ret

    .p2align 4, 0xcc
_probe:
    mov eax, dword ptr [esp + 4]
    mov eax, dword ptr [eax]
    and eax, 3
    ret

    .p2align 4, 0xcc
_report:
    mov eax, dword ptr [esp + 8]
    add dword ptr [_seen], eax
    ret

    .p2align 4, 0xcc
    .globl _start
_start:
    lea eax, [_unit]
    push eax
    call _pick
    add esp, 4
    ret

    .data
_seen:
    .long 0
_unit:
    .long 1, 2
