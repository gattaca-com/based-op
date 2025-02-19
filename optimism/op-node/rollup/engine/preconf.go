package engine

import (
	"fmt"

	"github.com/ethereum-optimism/optimism/op-service/eth"
)

// In charge of holding the current known preconf state and sending ready
// events to the engine api.
type PreconfHandler struct {
}

type PreconfChannels struct {
	EnvCh     chan *eth.SignedEnv
	NewFragCh chan *eth.SignedNewFrag
	SealCh    chan *eth.SignedSeal
}

// TODO: does this need a reference to the engine api?
func Start() PreconfChannels {
	channels := PreconfChannels{
		EnvCh:     make(chan *eth.SignedEnv),
		NewFragCh: make(chan *eth.SignedNewFrag),
		SealCh:    make(chan *eth.SignedSeal),
	}

	go preconfHandler(channels)
	return channels
}

// TODO: handle the actual events, not just their arrival.
func preconfHandler(channels PreconfChannels) {
	for {
		select {
		case <-channels.EnvCh:
			fmt.Println("Env received by the preconf handler")
		case <-channels.NewFragCh:
			fmt.Println("Frag receved by the preconf handler")
		case <-channels.SealCh:
			fmt.Println("Seal received by the preconf handler")
		}
	}
}
