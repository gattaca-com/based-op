package types

import (
	"encoding/json"
	"log"
	"math/big"

	"github.com/ethereum/go-ethereum/common"
	// "github.com/ethereum/go-ethereum/core/state"
)

type UnsealedBlock struct {
	Number             *big.Int
	Frag               []Frag
	LastSequenceNumber uint64
	Hash               common.Hash
	// State              *state.StateDB
}

func NewUnsealedBlock() *UnsealedBlock {
	return &UnsealedBlock{
		Number:             new(big.Int),
		Frag:               []Frag{},
		LastSequenceNumber: *new(uint64),
		Hash:               common.Hash{},
		// State:              nil,
	}
}

func (ub *UnsealedBlock) InsertNewFrag(frag Frag) {
	ub.Frag = append(ub.Frag, frag)
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
