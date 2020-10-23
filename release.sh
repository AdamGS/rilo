#!/usr/bin/env bash

cargo clean 2> /dev/null && cargo build --release 2> /dev/null

if [ $? -eq 0 ]; then
	chmod u+x target/release/rilo && echo "Release is successful!";
else
	return 1;
fi

