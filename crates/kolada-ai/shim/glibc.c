/* Linux (glibc) only. llama.cpp built on a glibc of 2.38 or later calls the
 * C23 forms of strtol and sscanf (__isoc23_*), which older glibcs (Debian
 * 12, Ubuntu 22.04) do not have, so the helper would not start there. They
 * are defined here on the classic forms, which differ only in reading a 0b
 * prefix that llama.cpp never parses. This file is compiled as C17 without
 * _GNU_SOURCE, so the calls below are the classic ones. */
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

long __isoc23_strtol(const char *s, char **end, int base) { return strtol(s, end, base); }
long long __isoc23_strtoll(const char *s, char **end, int base) { return strtoll(s, end, base); }
unsigned long __isoc23_strtoul(const char *s, char **end, int base) { return strtoul(s, end, base); }
unsigned long long __isoc23_strtoull(const char *s, char **end, int base) { return strtoull(s, end, base); }

int __isoc23_sscanf(const char *s, const char *format, ...) {
    va_list args;
    va_start(args, format);
    int n = vsscanf(s, format, args);
    va_end(args);
    return n;
}

int __isoc23_vsscanf(const char *s, const char *format, va_list args) { return vsscanf(s, format, args); }
