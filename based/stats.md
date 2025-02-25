# Based-op Performance 

Aside from providing a frictionless path towards becoming fully `based`, one of the development goals of `based-op` is to vastly scale up the achievable throughput of the chain.
This doc contains a collection of tests we utilize to gauge our progress towards the ["North-Star"](https://blog.base.dev/scaling-base-in-2025) of being able to sustain a throughput of 1 Giga Gas /s.
It will be continuously updated with the newest findings as we continue improving the `based-op` stack.

    All results outlined below have been achieved on an AMD 9950x cpu with 16 physical cores, 64gb RAM, a 7gb/s r/w NVME ssd

To summarise the current state of affairs:
  - Gateway: has arguably the heaviest task, but is already able to sustain 0.8-1.1 Ggas/s on realistic workloads.
  - OP-Reth Fallback: needs to stay synced so it can take over from the Gateway in case of failure. It has no issues syncing over 1.5 Ggas/s.
  - OP-Geth Follower/Gossip nodes: are in charge of serving and gossiping partially built blocks (`Frags`). So far tested up to 800Mgas/s with transfers.
  - Network as a whole: The biggest bottleneck at present is making the gossiping performant enough to keep up with the amount of data. Under active development

## Real world Sepolia gateway-only throughput

*Setup*:
  1. Partially synced up Base Sepolia database
  2. fetch `N` blocks starting from the last synced block number + 1
  3. decompose those blocks until we gather ~100k txs that landed on chain "in the future" from the pov of our local state
  4. feed this set of txs to the gateway over 2s timespans
  5. call `EngineApi` `getPayload` call and see how many txs and gas got included in the block

We posit that this gives us a relatively representative set of txs, mimicking real world on chain behavior if it were vastly scaled up compared to the current status quo.
We measure total block time from the `EngineApi` `ForkChoiceUpdate` call, signalling that the gateway should start sequencing, until the sealed `OpBlock`is returned.
Most notably, this includes the block seal time.
Notice that we chose to send the `GetPayload` request at 1.8s so we would not overrun the target 2s block time too severly. 

Starting from Sepolia block `21500198`, we reach a sustained rate of 900-950 MGas/s.
From the 103k txs, we sequence a total of 10-13k txs per block in \~2.22s.

This is a full end-to-end measurement, including:
  1. Greedy sorting every 200ms `Frag`
  2. Generating and sending the respective receipts and `Frags` to be served by the Follower `OP-Geth Node`
  3. Calculating roots and producing the full `OpBlock`.

In fact a large portion of time is spent on the state root which takes \~400ms on blocks of this size (\~2 GGas/block). 


## Minimum latency test
Our second test is with a testnet in kurtosis using the parameters of the current OP base chain, i.e. the standard 2s blocktime and 60 MGas/block constraints hold.
We incude the L1 chain, a sequencer and follower node, the portal, and gateway.

The objective of this test is to gain an indication of the lower bound of end-to-end latency of a single `eth_sendRawTransaction` to the corresponding succesful `eth_getReceipt`,
signalling that a `Frag` including the transaction was sealed and will land on chain.

We can leverage the fact that a `Frag` will only be sealed if there is at least one Transaction included in it after the target `Frag` time (200ms here).
Hence, if we send a single transaction after block time + 400ms, it should lead to the best case scenario in terms of Receipt latency.

To recap, we will time how long it takes for the following sequence of events to happen:
    `eth_rawTransaction` -> Portal -> Gateway -> Simulator -> included in Frag -> Frag seal -> successful `eth_getReceipt`

We find that this ideal scenario round trip measurement falls between 400 - 600 micro seconds.

## Kurtosis full test chain

In this test, we try to gauge the performance of the system as a whole.

*Setup*:
1. Recompile `op-deployer` and the `l1` and `l2` deployment contracts to set `GasLimit` to 60 Giga Gas per block.
2. Spin up the full kurtosis dev net locally, leaving out the blockscout
3. Fund 10000 random  accounts from the dev account
4. Start sending transactions between different accounts at varying rates

We are mainly interested in the performance of the `OP-Geth` follower node as it gets bombarded with `Frags` and full blocks to sync. 
The results are tabulated below.

### 35-40 MGas/s

| number | n_txs  | mgas    | elapsed    | mgas/s   |  
| 346    | 3659   |76.862   |  156.523ms |  491.056 |
| 347    | 3941   |82.784   |  163.130ms |  507.469 |
| 348    | 3601   |75.652   |  154.409ms |  489.944 |
| 349    | 3600   |75.628   |  154.092ms |  490.798 |
| 350    | 3601   |75.644   |  149.759ms |  505.103 |
| 351    | 3601   |75.644   |  148.862ms |  508.146 |
| 352    | 3601   |75.644   |  165.561ms |  456.893 |
| 353    | 3999   |84.002   |  163.353ms |  514.235 |
| 354    | 3601   |75.652   |  153.231ms |  493.711 |
| 355    | 3599   |75.607   |  151.434ms |  499.274 |
| 356    | 3601   |75.644   |  149.625ms |  505.555 |
| 357    | 3600   |75.623   |  148.254ms |  510.088 |
| 358    | 3601   |75.644   |  149.003ms |  507.665 |
| 359    | 4001   |84.044   |  158.500ms |  530.244 |
| 360    | 3600   |75.631   |  147.477ms |  512.831 |
| 361    | 3601   |75.649   |  157.010ms |  481.810 |
| 362    | 3601   |75.644   |  151.546ms |  499.146 |
| 363    | 3601   |75.644   |  158.719ms |  476.589 |
| 364    | 3601   |75.644   |  148.514ms |  509.336 |
| 365    | 4001   |84.044   |  163.933ms |  512.669 |
| 366    | 3600   |75.631   |  151.182ms |  500.263 |
| 367    | 3601   |75.649   |  146.997ms |  514.631 |
| 368    | 3601   |75.644   |  154.391ms |  489.949 |
| 369    | 3601   |75.644   |  152.247ms |  496.849 |
| 370    | 3600   |75.623   |  154.023ms |  490.982 |

### 80MGas/s
| number | n_txs  | mgas    | elapsed    | mgas/s   |  
| 39     | 6893   |144.776  | 263.994ms  | 548.405  |
| 40     | 5400   |113.423  | 212.473ms  | 533.820  |
| 41     | 5400   |113.431  | 211.135ms  | 537.243  |
| 42     | 4923   |103.411  | 192.799ms  | 536.367  |
| 43     | 5279   |110.882  | 198.231ms  | 559.355  |
| 44     | 5401   |113.444  | 204.259ms  | 555.390  |
| 45     | 5400   |113.423  | 214.677ms  | 528.340  |
| 46     | 5401   |113.444  | 198.874ms  | 570.430  |
| 47     | 5401   |113.452  | 204.165ms  | 555.689  |
| 48     | 5329   |111.937  | 196.696ms  | 569.088  |
| 49     | 4872   |102.335  | 195.134ms  | 524.434  |
| 50     | 5400   |113.423  | 203.026ms  | 558.660  |
| 51     | 5400   |113.423  | 196.789ms  | 576.365  |
| 52     | 5401   |113.444  | 199.982ms  | 567.270  |
| 53     | 5399   |113.410  | 198.765ms  | 570.572  |
| 54     | 5399   |113.407  | 214.802ms  | 527.962  |
| 55     | 4802   |100.865  | 197.828ms  | 509.860  |
| 56     | 5400   |113.423  | 198.248ms  | 572.125  |

### 125 MGas/s
| number | n_txs  | mgas    | elapsed    | mgas/s   |  
| 400    |  8000  |  168.023| 242.515ms  | 692.833  | 
| 401    |  8489  |  178.292| 238.541ms  | 747.424  | 
| 402    |  8513  |  178.804| 233.049ms  | 767.236  | 
| 403    |  7999  |  168.007| 227.751ms  | 737.680  | 
| 404    |  8000  |  168.023| 230.591ms  | 728.660  | 
| 405    |  9000  |  189.023| 250.031ms  | 755.997  | 
| 406    |  8000  |  168.023| 227.319ms  | 739.149  | 
| 407    |  8000  |  168.023| 241.663ms  | 695.275  | 
| 408    |  8531  |  179.182| 225.301ms  | 795.300  | 
| 409    |  8469  |  177.877| 255.304ms  | 696.726  | 
| 410    |  8001  |  168.044| 236.280ms  | 711.205  | 
| 411    |  8001  |  168.044| 247.208ms  | 679.766  | 
| 412    |  8999  |  189.002| 243.667ms  | 775.656  | 
| 413    |  8000  |  168.023| 246.257ms  | 682.305  | 
| 414    |  8000  |  168.031| 236.262ms  | 711.207  | 
| 415    |  8461  |  177.709| 249.158ms  | 713.238  | 
| 416    |  8540  |  179.363| 234.249ms  | 765.692  | 
| 417    |  8001  |  168.044| 243.097ms  | 691.262  | 
| 418    |  8000  |  168.023| 229.455ms  | 732.267  | 
| 419    |  8672  |  182.135| 254.595ms  | 715.390  | 
| 420    |  8328  |  174.919| 230.976ms  | 757.304  | 
| 421    |  8001  |  168.049| 245.126ms  | 685.562  | 
| 422    |  7999  |  168.002| 225.160ms  | 746.143  | 
| 423    |  9000  |  189.023| 237.129ms  | 797.129  | 
| 424    |  8001  |  168.044| 231.382ms  | 726.259  | 
| 425    |  8001  |  168.044| 227.999ms  | 737.036  | 
| 426    |  8427  |  176.998| 231.633ms  | 764.129  | 
| 427    |  8573  |  180.061| 236.082ms  | 762.706  | 
| 428    |  7999  |  168.002| 239.397ms  | 701.771  |

### 220 MGas/s
| number | n_txs | mgas    | elapsed   | mgas/s  |  
| 117    | 13650 | 286.673 | 307.870ms | 931.146 |
| 118    | 12802 | 268.865 | 294.923ms | 911.643 |
| 119    | 13197 | 277.160 | 307.994ms | 899.887 |
| 120    | 13776 | 289.327 | 304.911ms | 948.891 |
| 121    | 12226 | 256.774 | 294.424ms | 872.123 |
| 122    | 14001 | 294.044 | 302.356ms | 972.505 |
| 123    | 12592 | 264.455 | 290.532ms | 910.242 |
| 124    | 13408 | 281.591 | 297.049ms | 947.958 |
| 125    | 13372 | 280.835 | 302.324ms | 928.917 |
| 126    | 12628 | 265.219 | 292.269ms | 907.447 |
| 127    | 13997 | 293.965 | 311.264ms | 944.423 |
| 128    | 12016 | 252.359 | 312.709ms | 807.007 |
| 129    | 13984 | 293.687 | 317.754ms | 924.258 |
| 130    | 12001 | 252.044 | 290.399ms | 867.922 |
| 131    | 14001 | 294.044 | 302.628ms | 971.633 |
| 132    | 12781 | 268.432 | 316.552ms | 847.987 |
| 133    | 13216 | 277.564 | 329.163ms | 843.242 |
| 134    | 13619 | 286.022 | 299.538ms | 954.875 |
| 135    | 12380 | 260.003 | 296.550ms | 876.757 |
| 136    | 13997 | 293.960 | 321.323ms | 914.840 |

## Comments
With the current implementation of the gossiping network protocol we were only able to push the network as a whole up to a sustained rate of arond 220 MGas/s.
We are working to improve this.
In the meantime, the results so far appear promising: As the included gas per block increases, the throughput of the follower node actually improves.
This indicates the existence of a constant overhead per synced block, which becomes proportionally less impactful as the node spends more and more time on actually processing
transactions.

# Final words

While there are still some improvements to be made to the overall system, our outlook on reaching 1 GigaGas/s sustained throughput in the near future is bullish,
as supported by our stress testing results presented above.
