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

type Frag struct {
	BlockNumber uint64         `json:"blockNumber"`
	Seq         uint64         `json:"seq"`
	IsLast      bool           `json:"isLast"`
	Txs         []*Transaction `json:"txs"`
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
