# txspammer

`txspammer` is a command-line tool for performance testing of our Based-OP stack.

### **Quick Start Example**

This example demonstrates how to run the tool against the `based-op` stack, utilizing the dedicated sequencer and fragment stream endpoints for optimal performance and accurate latency measurement.

```bash
./txspammer \
    --eth_rpc.url "ws://<based-op-geth-ip>:8646" \
    --sequencer.url "http://<gateway-ip>:<rpc.port_no_auth>" \
    --fragstream.url "ws://<gateway-ip>:<rpc.port_ws>/state_stream" \
    --throughput 500 \
    --num_accounts 200
```

- `eth_rpc.url`: Should be the based-op-geth RPC endpoint. Can be either HTTP or WebSocket but prefer WebSocket for better performance.
- `sequencer.url`: Should be the gateway's rpc url. (with the port `--rpc.port_no_auth` on gateway launch parameter).
- `fragstream.url`: Should be the gateway's frag stream endpoint. (with the port `--rpc.port_ws` on gateway launch parameter)

### **Configuration Options**

Below is a detailed list of all available command-line options.

#### **Account & Funding**
*   `--root.private_key <ROOT_PRIVATE_KEY>`
    *   Private key of the root wallet used for funding worker accounts.
    *   **Default:** `0xac09...f2ff80` (Anvil/Hardhat default #0)

*   `--num_accounts <NUM_ACCOUNTS>`
    *   Number of worker accounts to generate and fund.
    *   **Default:** `100`

*   `--funding_amount <FUNDING_AMOUNT>`
    *   Amount of ETH to fund each worker account with.
    *   **Default:** `0.5`

#### **Transaction Profile**
*   `--throughput <THROUGHPUT>`
    *   Target transaction throughput in transactions per second (TPS).
    *   **Default:** `300`

*   `--tx_value <TX_VALUE>`
    *   Amount of ETH to transfer in each transaction (in Ether).
    *   **Default:** `0.0000000000000001`

*   `--gas_limit <GAS_LIMIT>`
    *   Gas limit for each transaction.
    *   **Default:** `21000`

*   `--max_fee_per_gas <MAX_FEE_PER_GAS>`
    *   Max fee per gas for EIP-1559 transactions (in Wei).
    *   **Default:** `1000000000` (1 Gwei)

*   `--max_priority_fee_per_gas <MAX_PRIORITY_FEE_PER_GAS>`
    *   Max priority fee per gas (miner tip) for EIP-1559 transactions (in Wei).
    *   **Default:** `20`

#### **Endpoint Configuration**

This is a critical section for configuring how `txspammer` interacts with the network.

*   `--eth_rpc.url <ETH_RPC_URL>`
    *   The JSON-RPC URL of the primary Ethereum node (HTTP or WebSocket). This endpoint is used for tasks like funding accounts and fetching chain data.
    *   **Default:** `http://127.0.0.1:8545`

*   `--sequencer.url <SEQUENCER_URL>`
    *   **Optional.** Specifies a direct URL to a sequencer or gateway for transaction submission (`eth_sendRawTransaction`).
    *   **Usage:** When set, transactions are sent directly to this URL, bypassing the general-purpose RPC node. This is the **recommended** approach for performance testing to ensure transactions are routed as efficiently as possible.
    *   If this option is not provided, transactions will be sent to the main `--eth_rpc.url`.

*   `--fragstream.url <FRAGSTREAM_URL>`
    *   **Optional, but highly recommended.** Specifies the WebSocket URL for the gateway's "frag stream".
    *   **Usage:** This stream provides real-time transaction receipts, which is the most efficient method for measuring end-to-end (E2E) latency.
    *   If this option is not provided, the tool will fall back to repeatedly polling `eth_getTransactionReceipt` via the `--eth_rpc.url`. This fallback method is inefficient, spams the RPC node with requests, and can result in less accurate latency measurements.
*   

### **Example Output**

```bash
> cargo run --release --bin txspammer -- --eth_rpc.url ws://127.0.0.1:8646 --sequencer.url http://127.0.0.1:9994 --throughput 1000 --fragstream.url ws://0.0.0.0:9999/state_stream --num_accounts 1000 --max_fee_per_gas 10000000000

    Finished `release` profile [optimized] target(s) in 0.32s
     Running `target/release/txspammer --eth_rpc.url 'ws://127.0.0.1:8646' --sequencer.url 'http://127.0.0.1:9994' --throughput 1000 --fragstream.url 'ws://0.0.0.0:9999/state_stream' --num_accounts 1000 --max_fee_per_gas 10000000000`
    
2025-09-17T13:15:16.325657Z  INFO Root account 0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266 has balance 8999.984420788457521880 eth
2025-09-17T13:15:18.032915Z  INFO All 1000 target accounts are funded and ready.              
2025-09-17T13:15:18.033807Z  INFO Last 5s: 0 tx confirmed, TPS: 0.00, Latency P50: 0.00s, P95: 0.00s, P99: 0.00s, txs<1s: 0.00%
2025-09-17T13:15:23.034161Z  INFO Last 5s: 4796 tx confirmed, TPS: 959.20, Latency P50: 0.11s, P95: 0.21s, P99: 0.28s, txs<1s: 100.00%
2025-09-17T13:15:28.034649Z  INFO Last 5s: 5088 tx confirmed, TPS: 1017.60, Latency P50: 0.10s, P95: 0.21s, P99: 0.26s, txs<1s: 100.00%
2025-09-17T13:15:33.035056Z  INFO Last 5s: 4906 tx confirmed, TPS: 981.20, Latency P50: 0.11s, P95: 0.23s, P99: 0.30s, txs<1s: 100.00%
2025-09-17T13:15:38.034324Z  INFO Last 5s: 5094 tx confirmed, TPS: 1018.80, Latency P50: 0.11s, P95: 0.21s, P99: 0.29s, txs<1s: 100.00%
2025-09-17T13:15:43.034137Z  INFO Last 5s: 4913 tx confirmed, TPS: 982.60, Latency P50: 0.11s, P95: 0.23s, P99: 0.29s, txs<1s: 100.00%
2025-09-17T13:15:48.034300Z  INFO Last 5s: 5088 tx confirmed, TPS: 1017.60, Latency P50: 0.11s, P95: 0.21s, P99: 0.28s, txs<1s: 100.00%
2025-09-17T13:15:53.034560Z  INFO Last 5s: 4925 tx confirmed, TPS: 985.00, Latency P50: 0.11s, P95: 0.24s, P99: 0.31s, txs<1s: 100.00%
2025-09-17T13:15:58.034724Z  INFO Last 5s: 5075 tx confirmed, TPS: 1015.00, Latency P50: 0.11s, P95: 0.21s, P99: 0.28s, txs<1s: 100.00%
2025-09-17T13:16:03.034927Z  INFO Last 5s: 4912 tx confirmed, TPS: 982.40, Latency P50: 0.11s, P95: 0.23s, P99: 0.29s, txs<1s: 100.00%
```