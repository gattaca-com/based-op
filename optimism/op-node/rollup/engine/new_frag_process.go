package engine

import (
	"github.com/ethereum-optimism/optimism/op-service/eth"
)

type NewFragProcessEvent struct {
	Frag *eth.Frag
}

func (ev NewFragProcessEvent) String() string {
	return "new-frag-process"
}

func (eq *EngDeriver) onNewFragProcess(ev NewFragProcessEvent) {
	eq.ec.engine.NewFrag(eq.ctx, ev.Frag)
	eq.log.Info("new fragment sent", "frag", ev.Frag)
}
