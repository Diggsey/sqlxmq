#! /usr/bin/env bash

set -e
devsupport=$(realpath $(dirname $0))
if [ ! -d "$PGDATA" ]; then
  initdb --no-instructions
fi
exec postgres -c listen_addresses= -c unix_socket_directories="${devsupport}/db_sockets"
