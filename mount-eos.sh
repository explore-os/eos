#!/bin/bash

for i in {1..5}; do
    sudo umount "$(eos mount)" &>/dev/null
done

mkdir -p "$(eos mount)" &>/dev/null

sudo mount -t 9p -o version=9p2000.L,trans=unix,uname=$USER "$(eos sock)" "$(eos mount)"
