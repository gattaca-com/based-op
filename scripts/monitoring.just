
# Invocation utils
self := "just -f " + justfile()

compose-file := canonicalize(join(justfile_directory(), "..", "monitoring", "compose.yml"))

# Start the monitoring service 
start:
    docker compose -f {{compose-file}} up -d

# Stop the monitoring service
stop:
    docker compose -f {{compose-file}} down
