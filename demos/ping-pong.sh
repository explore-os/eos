#!/bin/bash

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

ping_actor="ping"
pong_actor="pong"

actor_script="$SCRIPT_DIR/../examples/ping-pong.rn"

eos spawn --id ping "$actor_script"
eos spawn --id pong "$actor_script"

eos send --sender "$ping_actor" "$pong_actor" '{"state":"ping"}'
