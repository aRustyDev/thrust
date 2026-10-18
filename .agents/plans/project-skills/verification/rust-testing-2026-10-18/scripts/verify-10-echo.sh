#!/bin/bash
# What does `echo "\007\000\000" > file` write under bash, zsh and sh? And does `xxd` take more than one input file?
# Run: bash verify-10-echo.sh > ../logs/verify-10-echo.log 2>&1
T=$(mktemp -d)
for sh_name in bash zsh sh; do
  "$sh_name" -c "echo \"\\007\\000\\000\" > $T/echo-$sh_name.bin"
  printf '== %s\n' "$sh_name"
  xxd "$T/echo-$sh_name.bin"
done
printf '== printf (the portable choice)\n'
printf '\007\000\000' > "$T/printf.bin"
xxd "$T/printf.bin"
printf '== xxd with two input files (the second is treated as the OUTPUT file)\n'
printf 'AAAA' > "$T/in1.bin"
printf 'BBBB' > "$T/in2.bin"
xxd "$T/in1.bin" "$T/in2.bin"
printf '[xxd-two-args] exit=%s\n' "$?"
printf '== in2.bin after the call (was BBBB): '
xxd "$T/in2.bin"
rm -rf "$T"
