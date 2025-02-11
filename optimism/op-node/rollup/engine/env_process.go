package engine

import (
	"github.com/ethereum-optimism/optimism/op-service/eth"
)

type EnvProcessEvent struct {
	SignedEnv *eth.SignedEnv
}

func (ev EnvProcessEvent) String() string {
	return "env-frag-process"
}

func (eq *EngDeriver) onEnvProcess(ev EnvProcessEvent) {
	eq.ec.engine.Env(eq.ctx, ev.SignedEnv)
	eq.log.Info("new env sent", "env", ev.SignedEnv)
}
