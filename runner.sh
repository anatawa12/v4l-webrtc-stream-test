#!/bin/sh

REMOTE="interphone-2.remote"
EXEC="$1"
shift

scp "$EXEC" "$REMOTE":piterphone-pi-rs && ssh "$REMOTE" "./piterphone-pi-rs" "$@"
