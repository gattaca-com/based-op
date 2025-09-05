The chain replay driver is responsible for making the chain progress during
the chain replication testing, and checking its correctness.

After a chain replication testground has been spawned with the
spawner component, the driver will be responsible to send sequencer messages
in the p2p network and engine API calls to driver the chain forward. It is
essentially a bare-bone replacement for `op-node` which allows more control over
what's happening.

The driver during is lifecycle is responsible of the following:

1. Send the initial unsafe payload with block before chain replication, to kick
   off the follower (based)-op-nodes.
2. Send engine API messages to the gateway to control its state transition, and
   moving it into its sorting state
3. Send transactions to the Gateway, so that block production is triggered with
   the contents of the blocks we want to replay
4. Make followed based nodes create blocks and compare them with the provided
   verifier L2 EL.
