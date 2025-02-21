#!/bin/sh

set -e

geth init \
    --datadir=/data/geth/execution-data \
    --state.scheme=hash /network-configs/genesis-2151908.json

geth \
    --networkid=2151908 \
    --datadir=/data/geth/execution-data \
    --gcmode=archive \
    --state.scheme=hash \
    --http \
    --http.addr=0.0.0.0 \
    --http.vhosts=* \
    --http.corsdomain=* \
    --http.api=admin,engine,net,eth,web3,debug,miner \
    --ws \
    --ws.addr=0.0.0.0 \
    --ws.port=8546 \
    --ws.api=admin,engine,net,eth,web3,debug,miner \
    --ws.origins=* \
    --allow-insecure-unlock \
    --authrpc.port=8551 \
    --authrpc.addr=0.0.0.0 \
    --authrpc.vhosts=* \
    --authrpc.jwtsecret=/jwt/jwtsecret \
    --syncmode=full \
    --rpc.allow-unprotected-txs \
    --discovery.port=30303 \
    --port=30303 \
    --rollup.sequencerhttp=$SEQUENCER_URL \
    --bootnodes=$GETH_BOOTNODE
