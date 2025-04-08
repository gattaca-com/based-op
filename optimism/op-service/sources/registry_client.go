package sources

import (
	"context"
	"fmt"
	"slices"
	"sync"
	"time"

	"github.com/ethereum-optimism/optimism/op-node/rollup"
	"github.com/ethereum-optimism/optimism/op-service/client"
	"github.com/ethereum-optimism/optimism/op-service/sources/caching"
	"github.com/ethereum/go-ethereum/common"
	"github.com/ethereum/go-ethereum/log"
)

// RegistryClient provides typed bindings to retrieve registry data from an RPC source,
// caching results for future use.
type RegistryClient struct {
	ethClient *EthClient
	mu        sync.RWMutex
	// Map of block number to gateway for that block
	gateways           map[uint64]*RegistryResponse
	latestFetchedBlock uint64
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
		ethClient:          ethClient,
		mu:                 sync.RWMutex{},
		gateways:           make(map[uint64]*RegistryResponse),
		latestFetchedBlock: 0,
	}, nil
}

// GatewayForBlock returns the gateway for the given block number if it is cached
func (r *RegistryClient) GatewayForBlock(ctx context.Context, blockNumber uint64) (common.Address, error) {
	r.mu.RLock()
	defer r.mu.RUnlock()

	gateway, ok := r.gateways[blockNumber]
	if !ok {
		return common.Address{}, fmt.Errorf("gateway not cached for block number %d. Latest cached block number: %d", blockNumber, r.latestFetchedBlock)
	}

	return gateway.GatewayAddress, nil
}

// FetchNextNGateways fetches the current and next n gateways from the registry with a retry count
// Results are cached and can be retrieved by calling GatewayForBlock
func (r *RegistryClient) FetchNextNGateways(ctx context.Context, n uint64, maxRetries uint64) error {
	// Try at least once, then retry up to maxRetries times
	attempt := uint64(0)
	for {
		err := r.fetchNextNGatewaysSingle(ctx, n)
		if err == nil {
			return nil
		}

		if attempt >= maxRetries {
			return fmt.Errorf("failed to fetch gateways after %d attempts. Latest cached block number: %d: %w", attempt+1, r.latestFetchedBlock, err)
		}
		attempt++
	}
}

// fetchNextNGatewaysSingle fetches the current and next n gateways from the registry
func (r *RegistryClient) fetchNextNGatewaysSingle(ctx context.Context, n uint64) error {
	gateways := make([]*RegistryResponse, n+1)

	// Fetch current gateway
	current, err := r.ethClient.CurrentGateway(ctx)
	if err != nil {
		return err
	}
	gateways[0] = current

	// Fetch next n gateways
	for i := uint64(1); i <= n; i++ {
		gateway, err := r.ethClient.FutureGateway(ctx, i)
		if err != nil {
			return err
		}
		gateways[i] = gateway
	}

	// Update the cache
	r.updateGatewayInfo(gateways)

	// Cleanup old gateway entries
	r.cleanupOldGateways(100, 50)
	return nil
}

// cleanupOldGateways removes old gateway entries when the map exceeds maxSize
// keeping only the most recent entries up to targetSize
func (r *RegistryClient) cleanupOldGateways(maxSize, targetSize int) {
	r.mu.Lock()
	defer r.mu.Unlock()

	if len(r.gateways) <= maxSize {
		return
	}

	// Get sorted block numbers and remove oldest entries until targetSize
	blockNumbers := make([]uint64, 0, len(r.gateways))
	for num := range r.gateways {
		blockNumbers = append(blockNumbers, num)
	}
	slices.Sort(blockNumbers)

	for _, num := range blockNumbers[:len(blockNumbers)-targetSize] {
		delete(r.gateways, num)
	}
}

func (r *RegistryClient) updateGatewayInfo(results []*RegistryResponse) {
	r.mu.Lock()
	defer r.mu.Unlock()

	// update the cache for all results
	for _, result := range results {
		r.gateways[result.BlockNumber] = result

		// update the latest fetched block
		if result.BlockNumber > r.latestFetchedBlock {
			r.latestFetchedBlock = result.BlockNumber
		}
	}
}
