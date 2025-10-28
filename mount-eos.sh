#!/bin/bash

for i in {1..5}; do
    sudo umount "$(eos path mount)" &>/dev/null
done

mkdir -p "$(eos path mount)" &>/dev/null

sudo mount -t 9p -o version=9p2000.L,trans=unix,uname=$USER "$(eos path sock)" "$(eos path mount)"
