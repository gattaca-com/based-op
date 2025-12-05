import requests
import time

previous_balance = None
previous_at = time.time()

url = "http://localhost:7545"

token_address = "0x71a49e7ff0865d2de258e720782951879645df1b"
token_holder_address = "0x47Bae705382e91664F369d79aD8EcB8fDF23D355"
address = "0x0E2d15588e765f0ba315313C726041EA124e36CB"

def fetch_erc20_balance(address: str, token_address: str) -> float:
    # Remove '0x' prefix and pad address to 64 hex characters (32 bytes)
    address_clean = address.lower().replace('0x', '')
    address_padded = address_clean.zfill(64)
    
    # balanceOf(address) function selector is 0x70a08231
    # Format: function_selector (4 bytes) + address (32 bytes)
    data = f"0x70a08231{address_padded}"
    
    body = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_call",
        "params": [
            {"to": token_address, "data": data},
            "pending"
        ],
    }

    response = requests.post(url, json=body, timeout=5)
    response.raise_for_status()
    
    result = response.json()
    
    # Check for RPC errors
    if 'error' in result:
        raise Exception(f"RPC error: {result['error']}")
    
    result_hex = result.get('result', '0x0')
    if not result_hex or result_hex == '0x':
        return 0.0
    
    balance = int(result_hex, 16) / 1e18
    return balance

def fetch_balance(address: str) -> float:
    body = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_getBalance",
        "params": [address, "pending"],
    }
    response = requests.post(url, json=body, timeout=5)
    response.raise_for_status()
    
    result = response.json()
    
    # Check for RPC errors
    if 'error' in result:
        raise Exception(f"RPC error: {result['error']}")
    
    result_hex = result.get('result', '0x0')
    if not result_hex or result_hex == '0x':
        return 0.0
    
    balance = int(result_hex, 16) / 1e18
    return balance

while True:
    try:
        # balance = fetch_balance(address)
        balance = fetch_erc20_balance(token_holder_address, token_address)

        
        if previous_balance is None or balance != previous_balance:
            elapsed = time.time() - previous_at if previous_balance is not None else 0
            previous_at = time.time()
            if previous_balance is None:
                print(f"[{time.strftime('%Y-%m-%d %H:%M:%S')}] Initial balance: {balance:.10f} ETH")
            else:
                print(f"[{time.strftime('%Y-%m-%d %H:%M:%S')}] Balance changed: {previous_balance:.10f} ETH -> {balance:.10f} ETH (elapsed: {elapsed*1000:.1f}ms)")
            previous_balance = balance
    except Exception as e:
        print(f"Error fetching balance: {e}")
        time.sleep(1)  # Wait longer on error
        continue

    time.sleep(1/60)