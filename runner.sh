#!/bin/sh

REMOTE="interphone-2.remote"
EXEC="$1"
shift

rsync --progress "$EXEC" "$REMOTE":piterphone-pi-rs && exec ssh "$REMOTE" "./piterphone-pi-rs" "$@"
