import requests
import time

previous_balance = 0
previous_at = 0
while True:
    body = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_getBalance",
        "params": [
            "0x4D36DE6a194dDF98EE57323CfA3A45351d35e442",
            "latest"
        ],
    }
    try:
        response = requests.post("http://localhost:8645", json=body)
        balance = int(response.json().get('result', '0x0'), 16) / 1e18
    except Exception as e:
        balance = 0
        print(f"Error fetching balance: {e}")

    if balance != previous_balance:
        elapsed = time.time() - previous_at
        previous_at = time.time()
        print(f"[{time.strftime('%Y-%m-%d %H:%M:%S')}] Balance changed: {previous_balance:.10f} ETH -> {balance:.10f} ETH (elapsed: {elapsed*1000:.1f}ms)")
        previous_balance = balance

    time.sleep(1/60)