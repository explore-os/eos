#!/bin/bash

for i in {1..5}; do
    sudo umount /explore/system &>/dev/null
done

mkdir -p /explore/system &>/dev/null

sudo mount -t 9p -o version=9p2000.L,trans=unix,uname=$USER /tmp/eos-operator:0 /explore/system
