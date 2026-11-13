    .intel_syntax noprefix
    .text
    .globl @__security_check_cookie@4
@__security_check_cookie@4:
    ret
    .globl ___CxxFrameHandler3
___CxxFrameHandler3:
    xor eax, eax
    ret
    .data
    .globl ___security_cookie
___security_cookie:
    .long 0xbb40e64e
    .globl __fltused
__fltused:
    .long 0
    .text
    .globl __purecall
__purecall:
    ret
