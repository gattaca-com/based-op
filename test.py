import requests
import json
import time
import random

def get_node_info(op_geth_url):
    node_info_payload = json.loads('{"jsonrpc":"2.0","method":"admin_nodeInfo","params":[],"id":1}')
    response = requests.post(op_geth_url, json=node_info_payload)
    return response.json()

def get_head_block_hash(op_geth_url):
    node_info_payload = json.loads('{"jsonrpc":"2.0","method":"admin_nodeInfo","params":[],"id":1}')
    response = requests.post(op_geth_url, json=node_info_payload)
    head_block_hash = response.json()['result']['protocols']['eth']['head']
    return head_block_hash

def is_sequencer_running(op_node_url):
    sequencer_status_payload = json.loads('{"jsonrpc":"2.0","method":"admin_sequencerActive","params":[],"id":1}')
    response = requests.post(op_node_url, json=sequencer_status_payload)
    return response.json()['result']

def start_sequencer(op_node_url, op_geth_url):
    head_block_hash = get_head_block_hash(op_geth_url)
    start_payload = json.loads('{"jsonrpc":"2.0","method":"admin_startSequencer","params":[],"id":1}')
    start_payload['params'] = [head_block_hash]
    response = requests.post(op_node_url, json=start_payload)
    return response.json()

def stop_sequencer(op_node_url):
    stop_payload = json.loads('{"jsonrpc":"2.0","method":"admin_stopSequencer","params":[],"id":1}')
    response = requests.post(op_node_url, json=stop_payload)
    return response.json()

def sync_status(op_node_url):
    sync_payload = json.loads('{"jsonrpc":"2.0","method":"optimism_syncStatus","params":[],"id":1}')
    response = requests.post(op_node_url, json=sync_payload)
    return response.json()["result"]

def tx_pool():
    txpool_payload = json.loads('{"jsonrpc":"2.0","method":"txpool_content","params":[],"id":1}')
    response = requests.post(op_geth_url, json=txpool_payload)
    return response.json()

def geth_add_trusted_peer(op_geth_url, peer_url):
    add_trusted_peer_payload = json.loads('{"jsonrpc":"2.0","method":"admin_addTrustedPeer","params":["' + peer_url + '"],"id":1}')
    response = requests.post(op_geth_url, json=add_trusted_peer_payload)
    return response.json()

def geth_add_peer(op_geth_url, peer_url):
    add_peer_payload = json.loads('{"jsonrpc":"2.0","method":"admin_addPeer","params":["' + peer_url + '"],"id":1}')
    response = requests.post(op_geth_url, json=add_peer_payload)
    return response.json()

def geth_peers(op_geth_url):
    peers_payload = json.loads('{"jsonrpc":"2.0","method":"admin_peers","params":[],"id":1}')
    response = requests.post(op_geth_url, json=peers_payload)
    return response.json()


def eth_get_latest_block(op_geth_url):
    peers_payload = json.loads('{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["latest", false],"id":1}')
    response = requests.post(op_geth_url, json=peers_payload)
    return response.json()

def node_opp2p_self(op_node_url):
    opp2p_self_payload = json.loads('{"jsonrpc":"2.0","method":"opp2p_self","params":[],"id":1}')
    response = requests.post(op_node_url, json=opp2p_self_payload)
    return response.json()

def node_opp2p_peers(op_node_url):
    opp2p_peers_payload = json.loads('{"jsonrpc":"2.0","method":"opp2p_peers","params":[true],"id":1}')
    response = requests.post(op_node_url, json=opp2p_peers_payload)
    return response.json()

def node_opp2p_connect_peer(op_node_url, multiaddr):
    opp2p_connect_peer_payload = json.loads('{"jsonrpc":"2.0","method":"opp2p_connectPeer","params":["' + multiaddr + '"],"id":1}')
    response = requests.post(op_node_url, json=opp2p_connect_peer_payload)
    return response.json()

def node_opp2p_disconnect_peer(op_node_url, peer_id):
    opp2p_disconnect_peer_payload = json.loads('{"jsonrpc":"2.0","method":"opp2p_disconnectPeer","params":["' + peer_id + '"],"id":1}')
    response = requests.post(op_node_url, json=opp2p_disconnect_peer_payload)
    return response.json()

def node_opp2p_block_peer(op_node_url, peer_id):
    opp2p_block_peer_payload = json.loads('{"jsonrpc":"2.0","method":"opp2p_blockPeer","params":["' + peer_id + '"],"id":1}')
    response = requests.post(op_node_url, json=opp2p_block_peer_payload)
    return response.json()

def node_opp2p_unblock_peer(op_node_url, peer_id):
    opp2p_unblock_peer_payload = json.loads('{"jsonrpc":"2.0","method":"opp2p_unblockPeer","params":["' + peer_id + '"],"id":1}')
    response = requests.post(op_node_url, json=opp2p_unblock_peer_payload)
    return response.json()

def node_opp2p_blocked_peers(op_node_url):
    opp2p_blocked_peers_payload = json.loads('{"jsonrpc":"2.0","method":"opp2p_listBlockedPeers","params":[],"id":1}')
    response = requests.post(op_node_url, json=opp2p_blocked_peers_payload)
    return response.json()['result']

def node_disconnect_all_peers(op_node_urls):
    buff = [node_opp2p_peers(op_node_url)['result']['peers'] for op_node_url in op_node_urls]
    for i, op_node_url in enumerate(op_node_urls):
        for peer_id in buff[i]:
            node_opp2p_disconnect_peer(op_node_url, peer_id)
        print(f"Disconnected all peers from {op_node_url}. Total peers disconnected: {len(buff[i])}")

def node_block_all_peers(op_node_url):
    opp2p_peers = node_opp2p_peers(op_node_url)['result']['peers']
    for peer_id in opp2p_peers:
        node_opp2p_block_peer(op_node_url, peer_id)
    print(f"Blocked all peers from {op_node_url}. Total peers blocked: {len(opp2p_peers)}")

def node_unblock_all_peers(op_node_url):
    blocked_peers = node_opp2p_blocked_peers(op_node_url)
    for peer_id in blocked_peers:
        node_opp2p_unblock_peer(op_node_url, peer_id)
    print(f"Unblocked all peers from {op_node_url}. Total peers unblocked: {len(blocked_peers)}")

def portal_get_geth_enode(portal_url):
    portal_enode_payload = json.loads('{"jsonrpc":"2.0","method":"portal_opGethBootnodeEnode","params":[true],"id":1}')
    response = requests.post(portal_url, json=portal_enode_payload)
    return response.json()["result"]

def portal_get_node_multiaddr(portal_url):
    portal_multiaddr_payload = json.loads('{"jsonrpc":"2.0","method":"portal_opNodeGossipStatic","params":[true],"id":2}')
    response = requests.post(portal_url, json=portal_multiaddr_payload)
    return response.json()["result"]

def p2p_info():
    print(get_node_info("http://localhost:8545"))
    print(get_node_info("http://localhost:18545"))
    print(geth_add_trusted_peer("http://localhost:8545", "enode://7b69f600c082924e5b97c829638a8b068e24c270872e50b4a2656eb22eed811e0ea91121db3914c99637381b84bb5c76fef17965f26c441a79d01b957a0ea8de@127.0.0.1:40303"))
    print(geth_add_trusted_peer("http://localhost:18545", "enode://b77c6fd03c23e45fe27c6a7e4667f28875de158939bd4cfba2e79212492e2950201e170ff401160b1cfd698ed3d6dc2960141a51662e9e616e2e35870be7a572@127.0.0.1:30303"))
    print(geth_add_peer("http://localhost:8545", "enode://7b69f600c082924e5b97c829638a8b068e24c270872e50b4a2656eb22eed811e0ea91121db3914c99637381b84bb5c76fef17965f26c441a79d01b957a0ea8de@127.0.0.1:40303"))
    print(geth_add_peer("http://localhost:18545", "enode://b77c6fd03c23e45fe27c6a7e4667f28875de158939bd4cfba2e79212492e2950201e170ff401160b1cfd698ed3d6dc2960141a51662e9e616e2e35870be7a572@127.0.0.1:30303"))
    print(node_opp2p_self("http://localhost:9545"))
    print(node_opp2p_self("http://localhost:19545"))
    # print(node_opp2p_connect_peer("http://localhost:9545", "/ip4/127.0.0.1/tcp/19003/p2p/16Uiu2HAkvrWYPbbGfsyMFM5SSchcAgTfonDcpeqYJFh8Cw1M3XTf"))
    # time.sleep(2)
    # print(geth_peers(op_geth_url))
    # print(node_opp2p_peers(op_node_url))

def p2p_setup():
    address = node_opp2p_self(op_node_url2)['result']['addresses'][0]
    node_opp2p_connect_peer(op_node_url, address)
    print(f"Connecting to peer with address: {address}")

    address = node_opp2p_self(op_node_url)['result']['addresses'][0]
    node_opp2p_connect_peer(op_node_url2, address)
    print(f"Connecting to peer with address: {address}")

def monitor_sync_status():
    while True:
        time.sleep(0.5)
        try:
            print("==========================")
            print("")
            print("geth")
            print(get_node_info(op_geth_url))
            print("")
            print(get_node_info("http://localhost:18545"))
            print("node")
            print(sync_status(op_node_url))
            print("")
            print(sync_status("http://localhost:19545"))
        except Exception as e:
            print(f"Error fetching sync status: {e}")

def monitor_head():
    max_bn = 0
    min_bn = 0
    timer_sync_time = time.time()
    timer_block_time = time.time()
    block_times = []
    sync_delay = 0
    while True:
        time.sleep(0.01)
        try:
            node1_status = sync_status(op_node_url)
            node2_status = sync_status(op_node_url2)
            node1_head = node1_status["unsafe_l2"]["hash"]
            node2_head = node2_status["unsafe_l2"]["hash"]
            node1_block_number = node1_status["unsafe_l2"]["number"]
            node2_block_number = node2_status["unsafe_l2"]["number"]
            node1_seq = is_sequencer_running(op_node_url)
            node2_seq = is_sequencer_running(op_node_url2)
            max_block_number = max(node1_block_number, node2_block_number)
            min_block_number = min(node1_block_number, node2_block_number)

            if (max_bn != max_block_number):
                max_bn = max_block_number
                timer_sync_time = time.time()
                block_times.append(time.time() - timer_block_time)
                timer_block_time = time.time()
                if len(block_times) > 5:
                    block_times.pop(0)

            if (min_bn != min_block_number):
                min_bn = min_block_number
                sync_delay = time.time() - timer_sync_time

            node1_seq_indicator = '>' if node1_seq else ' '
            node2_seq_indicator = '>' if node2_seq else ' '

            node1_behind_indicator = '' if node1_block_number == max_block_number else '!'
            node2_behind_indicator = '' if node2_block_number == max_block_number else '!'

            if node1_head != node2_head:
                print("Heads are not equal!")
            print("\n"*100 + "======================================", flush=False)
            print(f"{node1_seq_indicator} Node 1 Head: {node1_head[2:12]}... ({node1_block_number}) {node1_behind_indicator}")
            print(f"{node2_seq_indicator} Node 2 Head: {node2_head[2:12]}... ({node2_block_number}) {node2_behind_indicator}")
            print("")
            print(f"Sync Delay: {int(sync_delay*1000)} ms")
            print(f"Block Times: {', '.join(f'{int(t*1000)}' for t in block_times)}")
            print("======================================", flush=False)
        except Exception as e:
            print(f"Error fetching head block hash: {e}")

def switch_sequencer():
    first_node_active = is_sequencer_running(op_node_url)
    stop_sequencer(op_node_url)
    stop_sequencer(op_node_url2)

    while True:
        node1_status = sync_status(op_node_url)
        node2_status = sync_status(op_node_url2)
        node1_head = node1_status["unsafe_l2"]["hash"]
        node2_head = node2_status["unsafe_l2"]["hash"]
        print(f"Node 1 Head: {node1_head}")
        print(f"Node 2 Head: {node2_head}")
        if node1_head != node2_head:
            print("Heads are not equal, waiting for sync...")
            p2p_setup()
            time.sleep(1)
        else:
            break

    if first_node_active:
        print("Starting sequencer on node 2...")
        response = start_sequencer(op_node_url2, op_geth_url2)
    else:
        print("Starting sequencer on node 1...")
        response = start_sequencer(op_node_url, op_geth_url)
    
    print(response)

op_node_url = "http://localhost:9545"
op_geth_url = "http://localhost:8545"
based_portal_url = "http://localhost:8080"

op_node_url2 = "http://localhost:19545"
op_geth_url2 = "http://localhost:18545"
based_portal_url2 = "http://localhost:18080"

def test_switching():
    tick = time.time()
    while True:
        current_time = time.time()
        elapsed_time = current_time - tick
        if elapsed_time > 15:
            tick += 15
            print(f"Switching sequencer at {time.strftime('%Y-%m-%d %H:%M:%S', time.localtime(tick))}")
            switch_sequencer()
        time.sleep(random.random())

def debug_portal_enode_enr():
    print(json.dumps(get_node_info(op_geth_url)))
    print(json.dumps(node_opp2p_self(op_node_url)))
    print("Geth Enode:", portal_get_geth_enode(based_portal_url))
    print("Node multiaddr:", portal_get_node_multiaddr(based_portal_url))

debug_portal_enode_enr()

# test_switching()
# node_block_all_peers(op_node_url)
# node_block_all_peers(op_node_url2)
# node_disconnect_all_peers(op_node_url)
# node_disconnect_all_peers(op_node_url2)
# while True:
#     print(eth_getlatestblock(op_geth_url))
#     print(eth_getlatestblock(op_geth_url2))
#     time.sleep(0.1)

# print(get_node_info(op_geth_url))
# print(get_node_info(op_geth_url2))

# print(geth_peers(op_geth_url2))


# print(node_opp2p_peers(op_node_url2))

# start_sequencer(op_node_url, op_geth_url)

# # monitor_sync_status()
# p2p_info()
# # print(stop_sequencer(op_node_url2))
monitor_head()

# while True:
#     match(input(">")):
#         case "start":
#             print("Starting sequencer...")
#             response = start_sequencer()
#             print(response)
#             continue
#         case "stop":
#             print("Stopping sequencer...")
#             response = stop_sequencer()
#             print(response)
#             continue
#         case "exit":
#             print("Exiting...")
#             continue
#         case "block":
#             node_block_all_peers(op_node_url)
#             node_block_all_peers(op_node_url2)
#             node_disconnect_all_peers(op_node_url)
#             node_disconnect_all_peers(op_node_url2)
#             continue
#         case "unblock":
#             node_unblock_all_peers(op_node_url)
#             node_unblock_all_peers(op_node_url2)
#             continue
#         case "pair":
#             node_unblock_all_peers(op_node_url)
#             node_unblock_all_peers(op_node_url2)
#             p2p_setup()
#             continue
#         case "unpair":
#             node_disconnect_all_peers([op_node_url, op_node_url2])
#             continue
#         case "switch":
#             switch_sequencer()
#             continue
#         case _:
#             print("Unknown command. Use 'start', 'stop', or 'exit'.")
#             continue