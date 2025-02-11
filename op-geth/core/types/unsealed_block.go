package types

import (
	"encoding/json"
	"log"
	"math/big"

	"github.com/ethereum/go-ethereum/common"
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
	} else {
		return *ub.LastSequenceNumber+1 == f.Seq
	}
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
		log.Fatalln(err)
		return err
	}

	f.BlockNumber = frag.BlockNumber
	f.Seq = frag.Seq
	f.IsLast = frag.IsLast

	for _, txData := range frag.Txs {
		var tx Transaction
		tx.UnmarshalBinary(txData)
		f.Txs = append(f.Txs, &tx)
	}

	return nil
}

type Env struct {
	Number      uint64         `json:number`
	Beneficiary common.Address `json:beneficiary`
	Timestamp   uint64         `json:timestamp`
	GasLimit    uint64         `json:gas_limit`
	Basefee     uint64         `json:basefee`
	Difficulty  *big.Int       `json:difficulty`
	Prevrandao  common.Hash    `json:prevrandao`
}
