package cli

import (
	"github.com/ethereum/go-ethereum/common"
	"github.com/urfave/cli/v2"

	"github.com/ethereum-optimism/optimism/op-node/flags"
)

// LoadGateway loads the gateway address from the CLI context.
func LoadGateway(ctx *cli.Context) *common.Address {
	if addr := ctx.String(flags.GatewayName); addr != "" {
		gatewayAddress := common.HexToAddress(addr)
		return &gatewayAddress
	}
	return nil
}
