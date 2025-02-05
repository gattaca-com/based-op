package engine

import (
	"github.com/ethereum-optimism/optimism/op-service/eth"
)

type SealFragSuccessEvent struct {
	Frag *eth.SealV0
}

func (ev SealFragSuccessEvent) String() string {
	return "new-frag-success"
}

func (eq *EngDeriver) onSealFragSuccess(ev SealFragSuccessEvent) {
	eq.log.Info("Sealed", ev.Frag)
}
