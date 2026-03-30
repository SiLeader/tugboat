#!/usr/bin/env bash

set -e

mkdir -p /run/flannel
echo "FLANNEL_NETWORK=${FLANNEL_NETWORK:-10.244.0.0/16}" > /run/flannel/subnet.env

exec "$@"
