#!/bin/bash

apk update
apk add openssl

echo -n 0x$(openssl rand -hex 32 | tr -d "\n") > /jwt/jwtsecret
