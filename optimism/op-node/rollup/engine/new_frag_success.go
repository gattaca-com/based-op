package engine

import (
	"github.com/ethereum-optimism/optimism/op-service/eth"
)

type NewFragSuccessEvent struct {
	Frag *eth.FragV0
}

func (ev NewFragSuccessEvent) String() string {
	return "new-frag-success"
}

func (eq *EngDeriver) onNewFragSuccess(ev NewFragSuccessEvent) {
	eq.log.Info("Inserted new frag", ev.Frag)
}
