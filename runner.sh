#!/bin/sh

REMOTE="interphone-2.remote"
EXEC="$1"
shift

rsync --progress "$EXEC" "$REMOTE":piterphone-pi-rs && exec ssh -t "$REMOTE" "./piterphone-pi-rs" "$@"
