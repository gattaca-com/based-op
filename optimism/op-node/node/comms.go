package node

import (
	"context"

	"github.com/libp2p/go-libp2p/core/peer"

	"github.com/ethereum-optimism/optimism/op-service/eth"
)

// Tracer configures the OpNode to share events
type Tracer interface {
	OnNewL1Head(ctx context.Context, sig eth.L1BlockRef)
	OnUnsafeL2Payload(ctx context.Context, from peer.ID, payload *eth.ExecutionPayloadEnvelope)
	OnPublishL2Payload(ctx context.Context, payload *eth.ExecutionPayloadEnvelope)
	OnNewFrag(ctx context.Context, from peer.ID, frag *eth.SignedNewFrag)
	OnPublishNewFrag(ctx context.Context, from peer.ID, frag *eth.SignedNewFrag)
	OnSealFrag(ctx context.Context, from peer.ID, frag *eth.SignedSeal)
	OnPublishSealFrag(ctx context.Context, from peer.ID, frag *eth.SignedSeal)
}

type noOpTracer struct{}

func (n noOpTracer) OnNewL1Head(ctx context.Context, sig eth.L1BlockRef) {}

func (n noOpTracer) OnUnsafeL2Payload(ctx context.Context, from peer.ID, payload *eth.ExecutionPayloadEnvelope) {
}

func (n noOpTracer) OnPublishL2Payload(ctx context.Context, payload *eth.ExecutionPayloadEnvelope) {}

func (n noOpTracer) OnNewFrag(ctx context.Context, from peer.ID, frag *eth.SignedNewFrag) {}

func (n noOpTracer) OnPublishNewFrag(ctx context.Context, from peer.ID, frag *eth.SignedNewFrag) {}

func (n noOpTracer) OnSealFrag(ctx context.Context, from peer.ID, frag *eth.SignedSeal) {}

func (n noOpTracer) OnPublishSealFrag(ctx context.Context, from peer.ID, frag *eth.SignedSeal) {}

var _ Tracer = (*noOpTracer)(nil)
