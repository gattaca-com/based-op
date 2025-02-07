package engine

import (
	"github.com/ethereum-optimism/optimism/op-service/eth"
)

type SealFragProcessEvent struct {
	SignedSeal *eth.SignedSeal
}

func (ev SealFragProcessEvent) String() string {
	return "new-frag-process"
}

func (eq *EngDeriver) onSealFragProcess(ev SealFragProcessEvent) {
	eq.ec.engine.SealFrag(eq.ctx, ev.SignedSeal)
	eq.log.Info("new seal sent", "seal", ev.SignedSeal)
}
