# Get Started

(TODO: some of these steps need to be automated first)

Starting the gateway with a testnet OP rollup is easy, thanks to Kurtosis  and the [Optimism Package](https://github.com/ethpandaops/optimism-package).

To start, clone the [repo](https://github.com/gattaca-com/based-op) and 


```yml
optimism_package:
  chains:
    - participants:
        - el_type: op-reth
          cl_type: op-node
          # cl_extra_params: [--rpc.based]
      mev_type: based-portal
      mev_params:
        based_portal_image: based_portal_local
        builder_host: "172.17.0.1"
        builder_port: "9997"
      additional_services:
        - blockscout
```


## Monitoring

### Grafana
Metrics and dashboard are WIP
