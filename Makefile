.PHONY: deps run clean restart help \
		build build-portal build-op-node build-op-geth \
		logs op-node-logs op-geth-logs \
		test-frag test-seal

.DEFAULT_GOAL := help

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

deps: ## 🚀 Install all dependencies
	# Kurtosis
	if [[ "$$(uname -s)" == "Darwin" ]]; then \
		xcode-select --install; \
		brew install kurtosis-tech/tap/kurtosis-cli; \
	elif [[ "$$(uname -s)" == "Linux" ]]; then \
		echo "deb [trusted=yes] https://apt.fury.io/kurtosis-tech/ /" | sudo tee /etc/apt/sources.list.d/kurtosis.list; \
		sudo apt update; \
		sudo apt install -y kurtosis-cli; \
	fi
	# Rust
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
	curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash

build: build-portal build-op-node build-op-geth ## 🏗️ Build

build-portal:
	docker buildx build -t based_portal_local --build-context reth=./reth ./based

build-op-node: ## 🏗️ Build OP node from optimism directory
	cd optimism && \
	IMAGE_TAGS=develop \
	PLATFORMS="linux/arm64" \
	docker buildx bake \
	-f docker-bake.hcl \
	op-node

build-op-geth: ## 🏗️ Build OP geth from op-eth directory
	docker build -t us-docker.pkg.dev/oplabs-tools-artifacts/images/op-geth ./op-geth

run: ## 🚀 Run
	kurtosis run optimism-package --args-file config.yml --enclave based-op

genesis:
	kurtosis files download based-op el_cl_genesis_data ./genesis

logs: ## 📜 Show logs
	kurtosis service logs -f based-op $(SERVICE)

based-portal-logs:
	$(MAKE) logs SERVICE=op-based-portal-1-op-kurtosis

op-node-logs:
	$(MAKE) logs SERVICE=op-cl-1-op-node-op-geth-op-kurtosis

op-geth-logs:
	$(MAKE) logs SERVICE=op-el-1-op-geth-op-node-op-kurtosis

clean: ## 🧹 Clean
	kurtosis enclave rm  based-op --force

restart: clean run ## 🔄 Restart

# Testing

test-frag:
	curl --request POST   --url http://localhost:$(PORT) --header 'Content-Type: application/json' \
	--data '{ \
		"jsonrpc": "2.0", \
		"id": 1, \
		"method": "based_newFrag", \
		"params": [ \
			{ \
				"signature": "0x1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890",  \
				"frag": { \
					"blockNumber": 2, \
					"seq": 1, \
					"isLast": false, \
					"txs": [], \
					"version": 0 \
				} \
			} \
		] \
	}'

test-seal:
	curl --request POST   --url http://localhost:$(PORT) --header 'Content-Type: application/json' \
	--data '{ \
		"jsonrpc": "2.0", \
		"id": 1, \
		"method": "based_sealFrag", \
		"params": [ \
			{ \
				"signature": "0x1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890",  \
				"seal": { \
					"totalFrags": 2, \
					"blockNumber": 2, \
					"gasUsed": 0, \
					"gasLimit": 0, \
					"parentHash": "0x1234567890123456789012345678901234567890123456789012345678901234", \
					"transactionsRoot": "0x1234567890123456789012345678901234567890123456789012345678901234", \
					"receiptsRoot": "0x1234567890123456789012345678901234567890123456789012345678901234", \
					"stateRoot": "0x1234567890123456789012345678901234567890123456789012345678901234", \
					"blockHash": "0x1234567890123456789012345678901234567890123456789012345678901234" \
				} \
			} \
		] \
	}'
