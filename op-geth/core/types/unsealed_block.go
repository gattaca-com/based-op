package types

import (
	"encoding/json"
	"log"
	"math/big"

	"github.com/ethereum/go-ethereum/common"
)

type UnsealedBlock struct {
	Number             *big.Int
	Frags              []Frag
	LastSequenceNumber uint64
	Hash               common.Hash

	Receipts Receipts
}

func NewUnsealedBlock(blockNumber *big.Int) *UnsealedBlock {
	return &UnsealedBlock{
		Number:             blockNumber,
		Frags:              []Frag{},
		LastSequenceNumber: *new(uint64),
		Hash:               common.Hash{},
		Receipts:           Receipts{},
	}
}

func IsOpened(ub *UnsealedBlock) bool {
	return ub != nil
}

func (ub *UnsealedBlock) IsNextFrag(f *Frag) bool {
	return ub.LastSequenceNumber+1 == f.Seq
}

type Frag struct {
	blockNumber uint64         `json:"blockNumber"`
	Seq         uint64         `json:"seq"`
	IsLast      bool           `json:"isLast"`
	Txs         []*Transaction `json:"txs"`
}

func (f *Frag) IsFirst() bool {
	return f.Seq == 0
}

func (f *Frag) BlockNumber() *big.Int {
	return new(big.Int).SetUint64(f.blockNumber)
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

	f.blockNumber = frag.BlockNumber
	f.Seq = frag.Seq
	f.IsLast = frag.IsLast

	for _, txData := range frag.Txs {
		var tx Transaction
		tx.UnmarshalBinary(txData)
		f.Txs = append(f.Txs, &tx)
	}

	return nil
}
