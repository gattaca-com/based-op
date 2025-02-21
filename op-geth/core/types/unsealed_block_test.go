package types

import (
	"testing"

	"github.com/ethereum/go-ethereum/common/hexutil"
)

func TestTxUnmarshal(t *testing.T) {
	raw, err := hexutil.Decode("0x02f8648203e8018203e8323294066827b2e69b655b8f30dd7f8d5c3c15ba2656d03280c080a02da0ebb1e25a76c4711c6ed36c6daa4135904e5de21ec84a2d482cf460358cfaa06044ba113d7c9612c1ab636b113cf76bc4c991148f1d714ef8646c9a90611ed0")
	if err != nil {
		panic(err)
	}

	tx := new(Transaction)
	err = tx.UnmarshalBinary(raw)

	if err != nil {
		panic(err)
	}
}
