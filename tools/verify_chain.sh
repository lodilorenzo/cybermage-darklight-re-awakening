#!/bin/bash
# Leaf-first verification: each stage disables the hooks of the callers so the
# functions under test are reached through their hooks.
# usage: tools/verify_chain.sh "NOHOOK1,..." "VERIFY1,..." [extra args]
NOHOOK="$1"; VERIFY="$2"; shift 2
CM_NO_HOOK="$NOHOOK" CM_VERIFY="$VERIFY" CM_VERIFY_EVERY=${EVERY:-3} timeout 2400 ./target/release/cybermage.exe --headless --seconds ${SECS:-30} --key 9:1C --hold 15:19:e48 --hold 19:21:e4d --hold 21:26:e48 "$@" 2>&1 | grep -E "^verify:|checked .* mismatched|MISMATCH" -A3 | head -40
