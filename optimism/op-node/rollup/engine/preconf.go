package engine

import (
	"context"
	"fmt"

	"github.com/ethereum-optimism/optimism/op-service/eth"
)

// Entrypoint to the PreconfHandler.
type PreconfChannels struct {
	EnvCh     chan *eth.SignedEnv
	NewFragCh chan *eth.SignedNewFrag
	SealCh    chan *eth.SignedSeal
}

func NewPreconfChannels() PreconfChannels {
	return PreconfChannels{
		EnvCh:     make(chan *eth.SignedEnv),
		NewFragCh: make(chan *eth.SignedNewFrag),
		SealCh:    make(chan *eth.SignedSeal),
	}
}

func (c *PreconfChannels) SendEnv(e *eth.SignedEnv)      { c.EnvCh <- e }
func (c *PreconfChannels) SendFrag(f *eth.SignedNewFrag) { c.NewFragCh <- f }
func (c *PreconfChannels) SendSeal(s *eth.SignedSeal)    { c.SealCh <- s }

type FragIndex struct {
	BlockNumber uint64
	Sequence    uint64
}

func index(f eth.NewFrag) FragIndex {
	return FragIndex{BlockNumber: f.BlockNumber, Sequence: f.Seq}
}

func (fi FragIndex) prev() FragIndex {
	return FragIndex{BlockNumber: fi.BlockNumber, Sequence: fi.Sequence - 1}
}

// In charge of holding the current known preconf state and sending ready
// events to the engine api. The events that are not ready yet will be held
// until they are.
type PreconfState struct {
	JustStarted  bool
	pendingEnvs  map[uint64]eth.Env
	sentEnvs     map[uint64]bool
	lastFragSent map[uint64]bool
	pendingFrags map[FragIndex]eth.NewFrag
	sentFrags    map[FragIndex]bool
	pendingSeals map[uint64]eth.Seal
	sentSeals    map[uint64]bool
}

func NewPreconfState() PreconfState {
	return PreconfState{
		JustStarted: true,
		pendingEnvs: make(map[uint64]eth.Env),
		sentEnvs:    make(map[uint64]bool),
	}
}

// Builds the preconf channels and starts a concurrent preconf handler in a separate goroutine.
func StartPreconf(ctx context.Context, e ExecEngine) PreconfChannels {
	channels := NewPreconfChannels()
	go preconfHandler(ctx, channels, e)
	return channels
}

// Checks if the state is new or if the previous block is sealed.
// Returns true if the env is ready to be sent to EL
func (s *PreconfState) putEnv(env eth.Env) bool {
	if s.JustStarted || s.sentSeals[env.Number-1] {
		s.sentEnvs[env.Number] = true
		s.JustStarted = false
		return true
	}
	s.pendingEnvs[env.Number] = env
	return false
}

// Checks if the frag is the first of the block and the env is present,
// or if the previous frag is sent.
// Returns true if the frag is ready to be sent to EL
func (s *PreconfState) putFrag(frag eth.NewFrag) bool {
	idx := index(frag)
	isFirst := frag.Seq == 0 && s.sentEnvs[frag.BlockNumber]
	previousSent := s.sentFrags[idx.prev()]
	if isFirst || previousSent {
		s.sentFrags[FragIndex{BlockNumber: frag.BlockNumber, Sequence: frag.Seq}] = true
		if frag.IsLast {
			s.lastFragSent[frag.BlockNumber] = true
		}
		return true
	}
	s.pendingFrags[idx] = frag
	return false
}

// Checks if the last frag of the block is sent.
// Returns true if the seal is ready to be sent to EL
func (s *PreconfState) putSeal(seal eth.Seal) bool {
	if s.lastFragSent[seal.BlockNumber] {
		s.sentSeals[seal.BlockNumber] = true
		return true
	}
	s.pendingSeals[seal.BlockNumber] = seal
	return true
}

// TODO: handle pending after something is put.
func preconfHandler(ctx context.Context, c PreconfChannels, e ExecEngine) {
	state := NewPreconfState()

	for {
		select {
		case env := <-c.EnvCh:
			fmt.Println("Env received by the preconf handler")
			if state.putEnv(env.Env) {
				e.Env(ctx, env)
			}
		case frag := <-c.NewFragCh:
			fmt.Println("Frag receved by the preconf handler")
			if state.putFrag(frag.Frag) {
				e.NewFrag(ctx, frag)
			}
		case seal := <-c.SealCh:
			fmt.Println("Seal received by the preconf handler")
			if state.putSeal(seal.Seal) {
				e.SealFrag(ctx, seal)
			}
		}
	}
}
