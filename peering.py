import requests
from itertools import combinations
import re

class NodePeering:
    def __init__(self, node_urls, geth_urls):
        self.node_urls = node_urls
        self.geth_urls = geth_urls

    def _json_rpc_request(self, url, method, params=None):
        payload = {
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params or []
        }
        response = requests.post(url, json=payload)
        response.raise_for_status()
        return response.json().get('result')

    def node_opp2p_self(self, op_node_url):
        return self._json_rpc_request(op_node_url, 'opp2p_self')

    def node_opp2p_connect_peer(self, op_node_url, multiaddr):
        return self._json_rpc_request(op_node_url, 'opp2p_connectPeer', [multiaddr])

    def geth_node_info(self, geth_node_url):
        return self._json_rpc_request(geth_node_url, 'admin_nodeInfo')
    
    def geth_add_peer(self, geth_node_url, enode):
        return self._json_rpc_request(geth_node_url, 'admin_addPeer', [enode])

    def p2p_setup(self):
        print("Starting P2P setup...")

        multi_addresses = {}

        for url in self.node_urls:
            self_info = self.node_opp2p_self(url)
            node_addresses = self_info.get('addresses', [])
            node_local = [re.sub(r'\/ip4\/[0-9\.]*\/', '/ip4/127.0.0.1/', addr) for addr in node_addresses]
            multi_addresses[url] = node_local
            print(f"Node at {url} has addresses: {node_addresses}")

        for (url1, addrs1), (url2, addrs2) in combinations(multi_addresses.items(), 2):
            if addrs1:
                for addr in addrs1:
                    print(f"Connecting Node at {url2} to Node at {url1} ({addr})")
                    self.node_opp2p_connect_peer(url2, addr)
            else:
                print(f"Could not get multiaddress for Node at {url1}.")

            if addrs2:
                for addr in addrs2:
                    print(f"Connecting Node at {url1} to Node at {url2} ({addr})")
                    self.node_opp2p_connect_peer(url1, addr)
            else:
                print(f"Could not get multiaddress for Node at {url2}.")

        enodes = {}

        for url in self.geth_urls:
            node_info = self.geth_node_info(url)
            enode = node_info.get('enode')
            enode_local = re.sub(r'@.+:', '@127.0.0.1:', enode)
            enodes[url] = enode_local
            print(f"Geth Node at {url} has enode: {enode_local}")

        for (url1, enode1), (url2, enode2) in combinations(enodes.items(), 2):
            if enode1:
                print(f"Adding Geth Node at {url1} to Node at {url2}")
                self.geth_add_peer(url2, enode1)
            else:
                print(f"Could not get enode for Geth Node at {url1}.")

            if enode2:
                print(f"Adding Geth Node at {url2} to Node at {url1}")
                self.geth_add_peer(url1, enode2)
            else:
                print(f"Could not get enode for Geth Node at {url2}.")

if __name__ == "__main__":
    node_urls = [
        "http://0.0.0.0:9545", # main node
        # "http://0.0.0.0:19545", # main node 2
        "http://0.0.0.0:8547" # follower node
    ]

    geth_urls = [
        "http://0.0.0.0:8545", # Geth node
        # "http://0.0.0.0:9545", # Geth node 2
        "http://0.0.0.0:8645" # Geth follower node
    ]

    p2p = NodePeering(node_urls, geth_urls)
    p2p.p2p_setup()
