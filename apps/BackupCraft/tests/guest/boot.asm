; boot.asm -- the test guest of the VM tests (vm_smoke.py): a boot sector that counts its boots
; in sector 2 of its disk and reports on the first serial port
;
;   nasm -f bin -o boot.bin boot.asm
;
; Serial output per boot: "BACKUPCRAFT-GUEST-OK boot=NNNN" (the count after this boot, hex).
; Then it halts.  Everything a VM test needs: a disk the guest writes, a text to wait for.
bits 16
org 0x7c00

start:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7c00
    sti
    mov [drive], dl
    ; COM1: 115200 8N1
    mov dx, 0x3f9
    xor al, al
    out dx, al
    mov dx, 0x3fb
    mov al, 0x80
    out dx, al
    mov dx, 0x3f8
    mov al, 1
    out dx, al
    mov dx, 0x3f9
    xor al, al
    out dx, al
    mov dx, 0x3fb
    mov al, 3
    out dx, al
    ; sector 2 -> 0x8000, count + 1, back
    mov ax, 0x0201
    mov cx, 0x0002
    xor dh, dh
    mov dl, [drive]
    mov bx, 0x8000
    int 0x13
    jc fail
    inc word [0x8000]
    mov ax, 0x0301
    mov cx, 0x0002
    xor dh, dh
    mov dl, [drive]
    mov bx, 0x8000
    int 0x13
    jc fail
    mov si, msg
    call print
    mov ax, [0x8000]
    call hex16
    mov si, crlf
    call print
halt:
    hlt
    jmp halt

fail:
    mov si, err
    call print
    jmp halt

; the zero-terminated string at si
print:
    lodsb
    test al, al
    jz .done
    call putc
    jmp print
.done:
    ret

; al to COM1 (waits for the transmitter)
putc:
    push ax
    mov dx, 0x3fd
.wait:
    in al, dx
    test al, 0x20
    jz .wait
    pop ax
    mov dx, 0x3f8
    out dx, al
    ret

; ax as four hex digits
hex16:
    mov cx, 4
.next:
    rol ax, 4
    push ax
    and al, 0x0f
    add al, '0'
    cmp al, '9'
    jbe .out
    add al, 7
.out:
    call putc
    pop ax
    loop .next
    ret

drive: db 0
msg:   db "BACKUPCRAFT-GUEST-OK boot=", 0
err:   db "BACKUPCRAFT-GUEST-DISKERROR", 13, 10, 0
crlf:  db 13, 10, 0

times 510 - ($ - $$) db 0
dw 0xaa55
