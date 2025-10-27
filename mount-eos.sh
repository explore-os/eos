#!/bin/bash
sudo mount -t 9p -o version=9p2000.L,trans=unix,uname=$USER /tmp/eos-operator:0 /explore/system
