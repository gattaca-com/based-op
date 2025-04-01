package sources

import (
	"time"

	"github.com/ethereum-optimism/optimism/op-node/rollup"
	"github.com/ethereum-optimism/optimism/op-service/client"
	"github.com/ethereum-optimism/optimism/op-service/sources/caching"
	"github.com/ethereum/go-ethereum/log"
)

// RegistryClient provides typed bindings to retrieve registry data from an RPC source,
type RegistryClient struct {
	*EthClient
}

func RegistryClientDefaultConfig(config *rollup.Config, kind RPCProviderKind) *EthClientConfig {
	// Cache 3/2 worth of sequencing window of receipts and txs
	span := int(config.SeqWindowSize) * 3 / 2
	return RegistryClientSimpleConfig(kind, span)
}

func RegistryClientSimpleConfig(kind RPCProviderKind, cacheSize int) *EthClientConfig {
	span := cacheSize
	if span > 1000 { // sanity cap. If a large sequencing window is configured, do not make the cache too large
		span = 1000
	}
	return &EthClientConfig{
		// receipts and transactions are cached per block
		ReceiptsCacheSize:     span,
		TransactionsCacheSize: span,
		HeadersCacheSize:      span,
		PayloadsCacheSize:     span,
		MaxRequestsPerBatch:   20, // TODO: tune batch param
		MaxConcurrentRequests: 10,
		TrustRPC:              true,
		MustBePostMerge:       false,
		RPCProviderKind:       kind,
		MethodResetDuration:   time.Minute,
	}
}

// NewRegistryClient wraps a RPC with bindings to fetch Registry data, while logging errors, tracking metrics (optional), and caching.
func NewRegistryClient(client client.RPC, log log.Logger, metrics caching.Metrics, config *EthClientConfig) (*RegistryClient, error) {
	ethClient, err := NewEthClient(client, log, metrics, config)
	if err != nil {
		return nil, err
	}

	return &RegistryClient{
		EthClient: ethClient,
	}, nil
}

// func (r *RegistryClient) CurrentGateway() common.Address {
// 	r.
// }
