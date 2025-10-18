#!/bin/sh
set -e

[ "$#" -eq 0 ] && set -- /usr/local/bin/supervisor
exec "$@"