package opnode

import (
	"context"

	"github.com/libp2p/go-libp2p/core/peer"

	"github.com/ethereum-optimism/optimism/op-node/node"
	"github.com/ethereum-optimism/optimism/op-service/eth"
)

type FnTracer struct {
	OnNewL1HeadFn        func(ctx context.Context, sig eth.L1BlockRef)
	OnUnsafeL2PayloadFn  func(ctx context.Context, from peer.ID, payload *eth.ExecutionPayloadEnvelope)
	OnPublishL2PayloadFn func(ctx context.Context, payload *eth.ExecutionPayloadEnvelope)
	OnNewFragFn          func(ctx context.Context, from peer.ID, frag *eth.SignedNewFrag)
	OnPublishNewFragFn   func(ctx context.Context, from peer.ID, frag *eth.SignedNewFrag)
	OnSealFn             func(ctx context.Context, from peer.ID, frag *eth.SignedSeal)
	OnPublishSealFn      func(ctx context.Context, from peer.ID, frag *eth.SignedSeal)
}

func (n *FnTracer) OnNewL1Head(ctx context.Context, sig eth.L1BlockRef) {
	if n.OnNewL1HeadFn != nil {
		n.OnNewL1HeadFn(ctx, sig)
	}
}

func (n *FnTracer) OnUnsafeL2Payload(ctx context.Context, from peer.ID, payload *eth.ExecutionPayloadEnvelope) {
	if n.OnUnsafeL2PayloadFn != nil {
		n.OnUnsafeL2PayloadFn(ctx, from, payload)
	}
}

func (n *FnTracer) OnPublishL2Payload(ctx context.Context, payload *eth.ExecutionPayloadEnvelope) {
	if n.OnPublishL2PayloadFn != nil {
		n.OnPublishL2PayloadFn(ctx, payload)
	}
}

func (n *FnTracer) OnNewFrag(ctx context.Context, from peer.ID, frag *eth.SignedNewFrag) {
	if n.OnNewFragFn != nil {
		n.OnNewFragFn(ctx, from, frag)
	}
}

func (n *FnTracer) OnPublishNewFrag(ctx context.Context, from peer.ID, frag *eth.SignedNewFrag) {
	if n.OnPublishNewFragFn != nil {
		n.OnPublishNewFragFn(ctx, from, frag)
	}
}

func (n *FnTracer) OnSealFrag(ctx context.Context, from peer.ID, seal *eth.SignedSeal) {
	if n.OnSealFn != nil {
		n.OnSealFn(ctx, from, seal)
	}
}

func (n *FnTracer) OnPublishSealFrag(ctx context.Context, from peer.ID, seal *eth.SignedSeal) {
	if n.OnPublishSealFn != nil {
		n.OnPublishSealFn(ctx, from, seal)
	}
}

var _ node.Tracer = (*FnTracer)(nil)
