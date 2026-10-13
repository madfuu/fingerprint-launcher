# PE fixtures

Minimal Windows executables whose only content is a `VERSIONINFO` resource with
`FILEVERSION 150,0,7871,186` (see `version.rc`). They test `src/kernel/probe/pe.rs` on every
host; nothing ever runs them.

| File                  | Format                          |
| --------------------- | ------------------------------- |
| `versioninfo-x64.bin` | PE32+ (x86-64), as `chrome.exe` |
| `versioninfo-x86.bin` | PE32 (i386)                     |

Generated on Linux with mingw-w64 (GCC 14):

```bash
echo 'void start(void) {}' > main.c

x86_64-w64-mingw32-windres version.rc -O coff -o version.o
x86_64-w64-mingw32-gcc -Os -s -nostdlib -nostartfiles -Wl,-e,start \
  -o versioninfo-x64.bin main.c version.o

i686-w64-mingw32-windres version.rc -O coff -o version32.o
i686-w64-mingw32-gcc -Os -s -nostdlib -nostartfiles -Wl,-e,_start \
  -o versioninfo-x86.bin main.c version32.o
```
