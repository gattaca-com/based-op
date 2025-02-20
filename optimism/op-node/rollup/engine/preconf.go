package engine

import (
	"context"
	"fmt"

	"github.com/ethereum-optimism/optimism/op-service/eth"
)

// In charge of holding the current known preconf state and sending ready
// events to the engine api. The events that are not ready yet will be held
// until they are.
type PreconfHandler struct {
}

// Entrypoint to the PreconfHandler.
type PreconfChannels struct {
	EnvCh     chan *eth.SignedEnv
	NewFragCh chan *eth.SignedNewFrag
	SealCh    chan *eth.SignedSeal
}

// Builds the preconf channels and starts a concurrent preconf handler in a separate goroutine.
func StartPreconf(ctx context.Context, e ExecEngine) PreconfChannels {
	channels := PreconfChannels{
		EnvCh:     make(chan *eth.SignedEnv),
		NewFragCh: make(chan *eth.SignedNewFrag),
		SealCh:    make(chan *eth.SignedSeal),
	}

	go preconfHandler(ctx, channels, e)
	return channels
}

func (c *PreconfChannels) SendEnv(e *eth.SignedEnv)      { c.EnvCh <- e }
func (c *PreconfChannels) SendFrag(f *eth.SignedNewFrag) { c.NewFragCh <- f }
func (c *PreconfChannels) SendSeal(s *eth.SignedSeal)    { c.SealCh <- s }

// TODO: add ordering. This is just a pass-through for now.
func preconfHandler(ctx context.Context, c PreconfChannels, e ExecEngine) {
	for {
		select {
		case env := <-c.EnvCh:
			fmt.Println("Env received by the preconf handler")
			e.Env(ctx, env)
		case frag := <-c.NewFragCh:
			fmt.Println("Frag receved by the preconf handler")
			e.NewFrag(ctx, frag)
		case seal := <-c.SealCh:
			fmt.Println("Seal received by the preconf handler")
			e.SealFrag(ctx, seal)
		}
	}
}
