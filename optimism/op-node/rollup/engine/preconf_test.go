package engine

import (
	"context"
	"encoding/hex"
	"math/big"
	"testing"

	"github.com/ethereum-optimism/optimism/op-service/eth"
	"github.com/ethereum/go-ethereum/common"
	"github.com/google/go-cmp/cmp"
)

type MockEngine struct {
	SeenEnvs     []eth.SignedEnv
	SeenNewFrags []eth.SignedNewFrag
	SeenSeals    []eth.SignedSeal
}

func (m *MockEngine) GetPayload(ctx context.Context, payloadInfo eth.PayloadInfo) (*eth.ExecutionPayloadEnvelope, error) {
	return nil, nil
}
func (m *MockEngine) ForkchoiceUpdate(ctx context.Context, state *eth.ForkchoiceState, attr *eth.PayloadAttributes) (*eth.ForkchoiceUpdatedResult, error) {
	return nil, nil
}
func (m *MockEngine) NewPayload(ctx context.Context, payload *eth.ExecutionPayload, parentBeaconBlockRoot *common.Hash) (*eth.PayloadStatusV1, error) {
	return nil, nil
}
func (m *MockEngine) L2BlockRefByLabel(ctx context.Context, label eth.BlockLabel) (eth.L2BlockRef, error) {
	var a eth.L2BlockRef
	return a, nil
}
func (m *MockEngine) NewFrag(ctx context.Context, frag *eth.SignedNewFrag) (*string, error) {
	m.SeenNewFrags = append(m.SeenNewFrags, *frag)
	return nil, nil
}
func (m *MockEngine) SealFrag(ctx context.Context, seal *eth.SignedSeal) (*string, error) {
	m.SeenSeals = append(m.SeenSeals, *seal)
	return nil, nil
}
func (m *MockEngine) Env(ctx context.Context, env *eth.SignedEnv) (*string, error) {
	m.SeenEnvs = append(m.SeenEnvs, *env)
	return nil, nil
}

func decodeOrPanic(s string) []byte {
	decoded, err := hex.DecodeString(s)
	if err != nil {
		panic(err)
	}
	return decoded
}

func decodeB20(s string) common.Address {
	return common.BytesToAddress(decodeOrPanic(s))
}

func env() eth.SignedEnv {
	return eth.SignedEnv{
		Signature: eth.Bytes65{0x01, 0x42, 0x65, 0x07, 0x01, 0x42, 0x65, 0x07, 0x01, 0x42, 0x65, 0x07},
		Env: eth.Env{
			Number:                1,
			Beneficiary:           decodeB20("1234567890123456789012345678901234567890"),
			Timestamp:             2,
			GasLimit:              3,
			Basefee:               4,
			Difficulty:            big.NewInt(5),
			Prevrandao:            common.BytesToHash(decodeOrPanic("e75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758")),
			ParentHash:            common.BytesToHash(decodeOrPanic("69c96d9de761cfc75fe9792b6166c758e75fae0065403d4091f3d6549c4219db")),
			ParentBeaconBlockRoot: common.BytesToHash(decodeOrPanic("c96d9de761cfc75fe9792b6166c758e75fae0065403d4091f3d6549c4219db69")),
			ExtraData:             []byte{0x01, 0x02, 0x03},
		},
	}
}

func frag() eth.SignedNewFrag {
	return eth.SignedNewFrag{
		Signature: eth.Bytes65{0x01, 0x42, 0x65, 0x07, 0x01, 0x42, 0x65, 0x07, 0x01, 0x42, 0x65, 0x07},
		Frag: eth.NewFrag{
			BlockNumber: 1,
			Seq:         0,
			IsLast:      false,
			Txs: [][]byte{
				{0x01, 0x02, 0x03},
				{0x04, 0x05, 0x06, 0x07},
			},
		},
	}
}

func seal() eth.SignedSeal {
	return eth.SignedSeal{
		Signature: eth.Bytes65{0x01, 0x42, 0x65, 0x07, 0x01, 0x42, 0x65, 0x07, 0x01, 0x42, 0x65, 0x07},
		Seal: eth.Seal{
			TotalFrags:       10,
			BlockNumber:      1,
			GasUsed:          30000,
			GasLimit:         60000,
			ParentHash:       eth.Bytes32{0x01, 0x03, 0x05},
			TransactionsRoot: eth.Bytes32{0x02, 0x03, 0x04, 0x7},
			ReceiptsRoot:     eth.Bytes32{0x00, 0x08},
			StateRoot:        eth.Bytes32{0xff, 0xfe, 0xfa},
			BlockHash:        eth.Bytes32{0xaa, 0xbb, 0xcc, 0xdd, 0xee},
		}}
}

func TestInOrder(t *testing.T) {
	// General setup
	var m MockEngine
	state := NewPreconfState(context.Background(), &m)

	// Data for the first block
	e := env()
	f := frag()
	f2 := f
	f2.Frag.Seq = 1
	f2.Frag.IsLast = true
	s := seal()

	state.putEnv(&e)
	if !cmp.Equal(m.SeenEnvs[0], e, cmp.AllowUnexported(big.Int{})) {
		t.Fatalf("The first env was not sent to the engine api.")
	}
	state.putFrag(&f)
	if !cmp.Equal(m.SeenNewFrags[0], f) {
		t.Fatalf("The first frag was not sent to the engine api")
	}
	state.putFrag(&f2)
	if !cmp.Equal(m.SeenNewFrags[1], f2) {
		t.Fatalf("The second frag was not sent to the engine api")
	}
	state.putSeal(&s)
	if !cmp.Equal(m.SeenSeals[0], s) {
		t.Fatalf("The first seal was not sent to the engine api")
	}

	// Second block, just to check that they don't collide with the first block events.
	e2 := e
	e2.Env.Number += 1
	f21 := f
	f21.Frag.BlockNumber = 2
	f21.Frag.Seq = 0
	f22 := f21
	f22.Frag.Seq = 1
	f23 := f21
	f23.Frag.Seq = 2
	f23.Frag.IsLast = true
	s2 := s
	s2.Seal.BlockNumber = 2

	state.putEnv(&e2)
	if !cmp.Equal(m.SeenEnvs[1], e2, cmp.AllowUnexported(big.Int{})) {
		t.Fatalf("The second env was not sent to the engine api.")
	}
	state.putFrag(&f21)
	if !cmp.Equal(m.SeenNewFrags[2], f21) {
		t.Fatalf("The first frag of the second block was not sent to the engine api")
	}
	state.putFrag(&f22)
	if !cmp.Equal(m.SeenNewFrags[3], f22) {
		t.Fatalf("The second frag of the second block was not sent to the engine api")
	}
	state.putFrag(&f23)
	if !cmp.Equal(m.SeenNewFrags[4], f23) {
		t.Fatalf("The second frag of the second block was not sent to the engine api")
	}
	state.putSeal(&s2)
	if !cmp.Equal(m.SeenSeals[1], s2) {
		t.Fatalf("The second seal was not sent to the engine api")
	}
}

func TestFragSealOutOfOrder(t *testing.T) {
	// General setup
	var m MockEngine
	state := NewPreconfState(context.Background(), &m)

	// Data for the first block. Same as the happy path.
	e := env()
	f := frag()

	state.putFrag(&f)
	if !cmp.Equal(len(m.SeenNewFrags), 0) {
		t.Fatalf("The first frag was pushed even if it shouldn't have been")
	}

	state.putEnv(&e)
	if !cmp.Equal(m.SeenEnvs[0], e, cmp.AllowUnexported(big.Int{})) {
		t.Fatalf("The first env was not sent to the engine api.")
	}
	// Now the frag should be pushed
	if !cmp.Equal(m.SeenNewFrags[0], f) {
		t.Fatalf("The first frag was not pushed even after the env was")
	}
}

func TestFragsOutOfOrder(t *testing.T) {
	// General setup
	var m MockEngine
	state := NewPreconfState(context.Background(), &m)

	// Data for the first block. Same as the happy path.
	e := env()
	f := frag()
	f2 := f
	f2.Frag.Seq = 1
	f2.Frag.IsLast = true

	state.putEnv(&e)
	if !cmp.Equal(m.SeenEnvs[0], e, cmp.AllowUnexported(big.Int{})) {
		t.Fatalf("The first env was not sent to the engine api.")
	}

	state.putFrag(&f2)
	if !cmp.Equal(len(m.SeenNewFrags), 0) {
		t.Fatalf("The second frag was pushed even if it shouldn't have been")
	}

	state.putFrag(&f)
	if !cmp.Equal(m.SeenNewFrags[0], f) {
		t.Fatalf("The first frag was not pushed even after the env was")
	}
	if !cmp.Equal(m.SeenNewFrags[1], f2) {
		t.Fatalf("The second frag was not pushed even after the env was")
	}
}

func TestSealBeforeLastFrag(t *testing.T) {
	// General setup
	var m MockEngine
	state := NewPreconfState(context.Background(), &m)

	// Data for the first block
	e := env()
	f := frag()
	f2 := f
	f2.Frag.Seq = 1
	f2.Frag.IsLast = true
	s := seal()

	state.putEnv(&e)
	if !cmp.Equal(m.SeenEnvs[0], e, cmp.AllowUnexported(big.Int{})) {
		t.Fatalf("The first env was not sent to the engine api.")
	}

	state.putFrag(&f)
	if !cmp.Equal(m.SeenNewFrags[0], f) {
		t.Fatalf("The first frag was not pushed even after the env was")
	}

	state.putSeal(&s)
	if !cmp.Equal(len(m.SeenSeals), 0) {
		t.Fatalf("The seal was pushed even if the last frag wasn't")
	}

	state.putFrag(&f2)
	if !cmp.Equal(m.SeenNewFrags[1], f2) {
		t.Fatalf("The second frag was not pushed even after the env was")
	}

	// The seal should be pushed now.
	if !cmp.Equal(m.SeenSeals[0], s) {
		t.Fatalf("The seal wasn't pushed even though the last frag was")
	}
}

// Similar to the in order test, but the first env is pushed at the end.
func TestEverythingOutOfOrderAllAtOnce(t *testing.T) {
	// General setup
	var m MockEngine
	state := NewPreconfState(context.Background(), &m)

	// Data for the first block
	e := env()
	f := frag()
	f2 := f
	f2.Frag.Seq = 1
	f2.Frag.IsLast = true
	s := seal()

	state.putFrag(&f)
	if !cmp.Equal(len(m.SeenNewFrags), 0) {
		t.Fatalf("The first frag was pushed without an env")
	}
	state.putFrag(&f2)
	if !cmp.Equal(len(m.SeenNewFrags), 0) {
		t.Fatalf("The second frag was pushed without an env")
	}
	state.putSeal(&s)
	if !cmp.Equal(len(m.SeenSeals), 0) {
		t.Fatalf("The block was sealed without an env")
	}

	state.putEnv(&e)
	if !cmp.Equal(m.SeenEnvs[0], e, cmp.AllowUnexported(big.Int{})) {
		t.Fatalf("The first env was not sent to the engine api.")
	}

	// Now the rest of the things should be pushed
	if !cmp.Equal(m.SeenNewFrags[0], f) {
		t.Fatalf("The first frag was not sent to the engine api")
	}
	if !cmp.Equal(m.SeenNewFrags[1], f2) {
		t.Fatalf("The second frag was not sent to the engine api")
	}
	if !cmp.Equal(m.SeenSeals[0], s) {
		t.Fatalf("The first seal was not sent to the engine api")
	}
}

func TestPreconfGapSavedByL2Block(t *testing.T) {
	var m MockEngine
	state := NewPreconfState(context.Background(), &m)

	// Data for the first block
	e := env()
	f := frag()
	f2 := f
	f2.Frag.Seq = 1
	f2.Frag.IsLast = true
	s := seal()

	// Data for the third block, this test case assumes the second is missing.
	e3 := e
	e3.Env.Number += 2
	f3 := f
	f3.Frag.BlockNumber += 2
	f3.Frag.IsLast = true
	s3 := s
	s3.Seal.BlockNumber += 2

	state.putEnv(&e)
	if !cmp.Equal(m.SeenEnvs[0], e, cmp.AllowUnexported(big.Int{})) {
		t.Fatalf("The first env was not sent to the engine api.")
	}

	state.putFrag(&f)
	if !cmp.Equal(m.SeenNewFrags[0], f) {
		t.Fatalf("The first frag was not pushed correctly")
	}
	state.putFrag(&f2)
	if !cmp.Equal(m.SeenNewFrags[1], f2) {
		t.Fatalf("The second frag was not pushed correctly")
	}
	state.putSeal(&s)
	if !cmp.Equal(m.SeenSeals[0], s) {
		t.Fatalf("The first seal was not pushed correctly")
	}

	// Now when we receive the third block it should not be pushed.
	state.putEnv(&e3)
	if !cmp.Equal(len(m.SeenEnvs), 1, cmp.AllowUnexported(big.Int{})) {
		t.Fatalf("The env for the third block shouldn't have been sent")
	}
	state.putFrag(&f3)
	if !cmp.Equal(len(m.SeenNewFrags), 2) {
		t.Fatalf("The frag for the third block shouldn't have been sent")
	}
	state.putSeal(&s3)
	if !cmp.Equal(len(m.SeenSeals), 1) {
		t.Fatalf("The seal for the third block shouldn't have been sent")
	}

	// Now we send an l2 block ref and everything from the third block should unblock.

	b := eth.L2BlockRef{
		Number: 3,
	}
	state.putL2Block(&b)
	if !cmp.Equal(m.SeenEnvs[1], e3, cmp.AllowUnexported(big.Int{})) {
		t.Fatalf("The env for the third block was not pushed correctly")
	}
	if !cmp.Equal(m.SeenNewFrags[2], f3) {
		t.Fatalf("The frag for the third block was not pushed correctly")
	}
	if !cmp.Equal(m.SeenSeals[1], s3) {
		t.Fatalf("The seal for the third block was not pushed correctly")
	}
}
