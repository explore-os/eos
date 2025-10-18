#!/bin/bash

pushd "$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )" &>/dev/null

ping_actor="/explore/actors/ping"
pong_actor="/explore/actors/pong"

eos spawn --id ping script ping-pong.rn
eos spawn --id pong script ping-pong.rn

eos send --sender "$ping_actor" "$pong_actor" '{"ping":true}'