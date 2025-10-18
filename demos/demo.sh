#!/bin/bash

ping_actor="/explore/actors/ping"
pong_actor="/explore/actors/pong"

actor_script="/explore/examples/ping-pong.rn"

eos spawn --id ping script "$actor_script"
eos spawn --id pong script "$actor_script

eos send --sender "$ping_actor" "$pong_actor" '{"state":"ping"}'