#!/bin/bash
cd "$(dirname "$0")"; S=$1; mkdir -p $S/lc $S/dump
ls $S/v | grep -v '\.xmp$' | xargs -P ${J:-4} -n 1 ./one.sh $S
