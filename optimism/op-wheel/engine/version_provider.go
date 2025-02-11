package engine

import (
	"strconv"

	"github.com/ethereum-optimism/optimism/op-service/eth"
)

type StaticVersionProvider int

func (v StaticVersionProvider) ForkchoiceUpdatedVersion(*eth.PayloadAttributes) eth.EngineAPIMethod {
	switch int(v) {
	case 1:
		return eth.FCUV1
	case 2:
		return eth.FCUV2
	case 3:
		return eth.FCUV3
	default:
		panic("invalid Engine API version: " + strconv.Itoa(int(v)))
	}
}

func (v StaticVersionProvider) NewPayloadVersion(uint64) eth.EngineAPIMethod {
	switch int(v) {
	case 1, 2:
		return eth.NewPayloadV2
	case 3:
		return eth.NewPayloadV3
	default:
		panic("invalid Engine API version: " + strconv.Itoa(int(v)))
	}
}

func (v StaticVersionProvider) GetPayloadVersion(uint64) eth.EngineAPIMethod {
	switch int(v) {
	case 1, 2:
		return eth.GetPayloadV2
	case 3:
		return eth.GetPayloadV3
	default:
		panic("invalid Engine API version: " + strconv.Itoa(int(v)))
	}
}

func (v StaticVersionProvider) NewFragVersion(uint64) eth.EngineAPIMethod {
	return eth.NewFragV0
}

func (v StaticVersionProvider) SealFragVersion(uint64) eth.EngineAPIMethod {
	return eth.SealFragV0
}

func (v StaticVersionProvider) EnvVersion(uint64) eth.EngineAPIMethod {
	return eth.EnvV0
}
