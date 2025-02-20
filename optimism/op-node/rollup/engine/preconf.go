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

func (fi FragIndex) next() FragIndex {
	return FragIndex{BlockNumber: fi.BlockNumber, Sequence: fi.Sequence + 1}
}

// In charge of holding the current known preconf state and sending ready
// events to the engine api. The events that are not ready yet will be held
// until they are.
type PreconfState struct {
	JustStarted  bool
	pendingEnvs  map[uint64]eth.SignedEnv
	sentEnvs     map[uint64]bool
	lastFragSent map[uint64]bool
	pendingFrags map[FragIndex]eth.SignedNewFrag
	sentFrags    map[FragIndex]bool
	pendingSeals map[uint64]eth.SignedSeal
	sentSeals    map[uint64]bool
	ctx          context.Context
	e            ExecEngine
}

func NewPreconfState(ctx context.Context, e ExecEngine) PreconfState {
	return PreconfState{
		JustStarted:  true,
		pendingEnvs:  make(map[uint64]eth.SignedEnv),
		sentEnvs:     make(map[uint64]bool),
		lastFragSent: make(map[uint64]bool),
		pendingFrags: make(map[FragIndex]eth.SignedNewFrag),
		sentFrags:    make(map[FragIndex]bool),
		pendingSeals: make(map[uint64]eth.SignedSeal),
		sentSeals:    make(map[uint64]bool),
		ctx:          ctx,
		e:            e,
	}
}

// Builds the preconf channels and starts a concurrent preconf handler in a separate goroutine.
func StartPreconf(ctx context.Context, e ExecEngine) PreconfChannels {
	channels := NewPreconfChannels()
	go preconfHandler(ctx, channels, e)
	return channels
}

// Checks if the state is new or if the previous block is sealed.
func (s *PreconfState) putEnv(sEnv *eth.SignedEnv) {
	env := sEnv.Env
	if s.JustStarted || s.sentSeals[env.Number-1] {
		s.sentEnvs[env.Number] = true
		s.JustStarted = false
		s.e.Env(s.ctx, sEnv)

		// When an env is sent we should check if we have the first frag of the block and put it.
		nextIndex := FragIndex{BlockNumber: env.Number, Sequence: 0}
		nextFrag, ok := s.pendingFrags[nextIndex]
		if ok {
			delete(s.pendingFrags, nextIndex)
			s.putFrag(&nextFrag)
		}
	} else {
		s.pendingEnvs[env.Number] = *sEnv
	}
}

// Checks if the frag is the first of the block and the env is present,
// or if the previous frag is sent.
func (s *PreconfState) putFrag(sFrag *eth.SignedNewFrag) {
	frag := sFrag.Frag
	idx := index(frag)
	isFirst := frag.Seq == 0 && s.sentEnvs[frag.BlockNumber]
	previousSent := s.sentFrags[idx.prev()]
	if isFirst || previousSent {
		s.sentFrags[idx] = true
		s.e.NewFrag(s.ctx, sFrag)

		// When a frag is sent we should check if the next is present or if the seal is present
		if frag.IsLast {
			s.lastFragSent[idx.BlockNumber] = true
			nextSeal, ok := s.pendingSeals[idx.BlockNumber]
			if ok {
				delete(s.pendingSeals, idx.BlockNumber)
				s.putSeal(&nextSeal)
			}
		} else {
			nextFrag, ok := s.pendingFrags[idx.next()]
			if ok {
				delete(s.pendingFrags, idx.next())
				s.putFrag(&nextFrag)
			}
		}
	} else {
		s.pendingFrags[idx] = *sFrag
	}
}

// Checks if the last frag of the block is sent.
func (s *PreconfState) putSeal(sSeal *eth.SignedSeal) {
	seal := sSeal.Seal
	if s.lastFragSent[seal.BlockNumber] {
		s.sentSeals[seal.BlockNumber] = true
		s.e.SealFrag(s.ctx, sSeal)
		// When we put a seal we should check if the env of the next is present.
		nextEnv, ok := s.pendingEnvs[seal.BlockNumber+1]
		if ok {
			delete(s.pendingEnvs, seal.BlockNumber+1)
			s.putEnv(&nextEnv)
		}
	} else {
		s.pendingSeals[seal.BlockNumber] = *sSeal
	}
}

// TODO: handle pending after something is put.
func preconfHandler(ctx context.Context, c PreconfChannels, e ExecEngine) {
	state := NewPreconfState(ctx, e)

	for {
		select {
		case env := <-c.EnvCh:
			fmt.Println("Env received by the preconf handler")
			state.putEnv(env)
		case frag := <-c.NewFragCh:
			fmt.Println("Frag receved by the preconf handler")
			state.putFrag(frag)
		case seal := <-c.SealCh:
			fmt.Println("Seal received by the preconf handler")
			state.putSeal(seal)
		}
	}
}
