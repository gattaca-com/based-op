package types

import (
	"bytes"
	"encoding/json"
	"math/big"

	"github.com/ethereum/go-ethereum/common"
	"github.com/ethereum/go-ethereum/common/hexutil"
	"github.com/ethereum/go-ethereum/log"
)

type UnsealedBlock struct {
	Env                *Env
	Frags              []Frag
	LastSequenceNumber *uint64
	Hash               common.Hash

	Receipts Receipts
}

func NewUnsealedBlock(e *Env) *UnsealedBlock {
	return &UnsealedBlock{
		Env:                e,
		Frags:              []Frag{},
		LastSequenceNumber: nil,
		Hash:               common.Hash{},
		Receipts:           Receipts{},
	}
}

func IsOpened(ub *UnsealedBlock) bool {
	return ub != nil
}

func (ub *UnsealedBlock) IsEmpty() bool {
	return len(ub.Frags) == 0
}

func (ub *UnsealedBlock) IsNextFrag(f *Frag) bool {
	if ub.LastSequenceNumber == nil {
		return f.IsFirst()
	}

	lastKnownFrag := ub.Frags[*ub.LastSequenceNumber]

	if lastKnownFrag.IsLast {
		return false
	} else {
		return lastKnownFrag.Seq+1 == f.Seq
	}
}

func (ub *UnsealedBlock) Transactions() []*Transaction {
	txs := []*Transaction{}
	for _, frag := range ub.Frags {
		txs = append(txs, frag.Txs...)
	}
	return txs
}

func (ub *UnsealedBlock) ByteTransactions() [][]byte {
	buffer := new(bytes.Buffer)
	txs := [][]byte{}
	for _, tx := range ub.Transactions() {
		buffer.Reset()
		tx.inner.encode(buffer)
		txs = append(txs, buffer.Bytes())
	}
	return txs
}

type Frag struct {
	BlockNumber uint64         `json:"blockNumber"`
	Seq         uint64         `json:"seq"`
	IsLast      bool           `json:"isLast"`
	Txs         []*Transaction `json:"txs"`
}

func (f *Frag) IsFirst() bool {
	return f.Seq == 0
}

func (f *Frag) UnmarshalJSON(data []byte) error {
	var frag struct {
		BlockNumber uint64
		Seq         uint64
		IsLast      bool
		Txs         [][]byte
	}

	if err := json.Unmarshal(data, &frag); err != nil {
		log.Error("Failed to unmarshal frag intermediate struct", "err", err)
		return err
	}

	f.BlockNumber = frag.BlockNumber
	f.Seq = frag.Seq
	f.IsLast = frag.IsLast

	for _, txData := range frag.Txs {
		var tx Transaction
		err := tx.UnmarshalBinary(txData)
		if err != nil {
			log.Error("Failed to unmarshal transaction", "err", err)
			return err
		}
		f.Txs = append(f.Txs, &tx)
	}

	return nil
}

func (f *Frag) MarshalJSON() ([]byte, error) {
	txs := make([][]byte, len(f.Txs))
	for i, tx := range f.Txs {
		if tx, err := tx.MarshalBinary(); err != nil {
			return nil, err
		} else {
			txs[i] = tx
		}
	}

	return json.Marshal(struct {
		BlockNumber uint64   `json:"blockNumber"`
		Seq         uint64   `json:"seq"`
		IsLast      bool     `json:"isLast"`
		Txs         [][]byte `json:"txs"`
	}{
		BlockNumber: f.BlockNumber,
		Seq:         f.Seq,
		IsLast:      f.IsLast,
		Txs:         txs,
	})
}

type Env struct {
	Number           uint64
	Beneficiary      common.Address
	Timestamp        uint64
	GasLimit         uint64
	Basefee          uint64
	Difficulty       *big.Int
	Prevrandao       common.Hash
	ParentHash       common.Hash
	ParentBeaconRoot common.Hash
	ExtraData        []byte
}

func (e *Env) UnmarshalJSON(data []byte) error {
	var env struct {
		Number           uint64         `json:"number"`
		Beneficiary      common.Address `json:"beneficiary"`
		Timestamp        uint64         `json:"timestamp"`
		GasLimit         uint64         `json:"gasLimit"`
		Basefee          uint64         `json:"basefee"`
		Difficulty       *hexutil.Big   `json:"difficulty"`
		Prevrandao       common.Hash    `json:"prevrandao"`
		ParentHash       common.Hash    `json:"parentHash"`
		ParentBeaconRoot common.Hash    `json:"parentBeaconRoot"`
		ExtraData        []byte         `json:"extraData"`
	}

	if err := json.Unmarshal(data, &env); err != nil {
		return err
	}

	e.Number = env.Number
	e.Beneficiary = env.Beneficiary
	e.Timestamp = env.Timestamp
	e.GasLimit = env.GasLimit
	e.Basefee = env.Basefee
	e.Difficulty = env.Difficulty.ToInt()
	e.Prevrandao = env.Prevrandao
	e.ParentHash = env.ParentHash
	e.ParentBeaconRoot = env.ParentBeaconRoot
	e.ExtraData = env.ExtraData

	return nil
}

func (e *Env) MarshalJSON() ([]byte, error) {
	return json.Marshal(struct {
		Number           uint64         `json:"number"`
		Beneficiary      common.Address `json:"beneficiary"`
		Timestamp        uint64         `json:"timestamp"`
		GasLimit         uint64         `json:"gasLimit"`
		Basefee          uint64         `json:"basefee"`
		Difficulty       *hexutil.Big   `json:"difficulty"`
		Prevrandao       common.Hash    `json:"prevrandao"`
		ParentHash       common.Hash    `json:"parentHash"`
		ParentBeaconRoot common.Hash    `json:"parentBeaconRoot"`
		ExtraData        []byte         `json:"extraData"`
	}{
		Number:           e.Number,
		Beneficiary:      e.Beneficiary,
		Timestamp:        e.Timestamp,
		GasLimit:         e.GasLimit,
		Basefee:          e.Basefee,
		Difficulty:       (*hexutil.Big)(e.Difficulty),
		Prevrandao:       e.Prevrandao,
		ParentHash:       e.ParentHash,
		ParentBeaconRoot: e.ParentBeaconRoot,
		ExtraData:        e.ExtraData,
	})
}
