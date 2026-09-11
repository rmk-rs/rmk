# Flash one half over J-Link, then stream its defmt log.
# With both halves' probes attached, add `SelectEmuBySN <serial>` below so
# JLinkExe does not pick one for you.
cp $1 $1.elf

ELF_FILE="$1.elf"

JLinkExe <<EOF
Device nrf52833_xxaa
SelectInterface SWD
Speed 4000
LoadFile ${ELF_FILE}
r
g
q
EOF

defmt-print -e $1 tcp
