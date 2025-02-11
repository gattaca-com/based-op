ethereum_package_shared_utils = import_module(
    "github.com/ethpandaops/ethereum-package/src/shared_utils/shared_utils.star"
)

ethereum_package_el_context = import_module(
    "github.com/ethpandaops/ethereum-package/src/el/el_context.star"
)
ethereum_package_el_admin_node_info = import_module(
    "github.com/ethpandaops/ethereum-package/src/el/el_admin_node_info.star"
)

ethereum_package_node_metrics = import_module(
    "github.com/ethpandaops/ethereum-package/src/node_metrics_info.star"
)
ethereum_package_constants = import_module(
    "github.com/ethpandaops/ethereum-package/src/package_io/constants.star"
)

ethereum_package_input_parser = import_module(
    "github.com/ethpandaops/ethereum-package/src/package_io/input_parser.star"
)

constants = import_module("../../package_io/constants.star")
observability = import_module("../../observability/observability.star")
util = import_module("../../util.star")

RPC_PORT_NUM = 8545
# ENGINE_RPC_PORT_NUM = 9551

# The min/max CPU/memory that the execution node can use
EXECUTION_MIN_CPU = 100
EXECUTION_MIN_MEMORY = 256

# Port IDs
RPC_PORT_ID = "rpc"
# ENGINE_RPC_PORT_ID = "engine-rpc"

# Paths
# METRICS_PATH = "/metrics"

# The dirpath of the execution data directory on the client container
EXECUTION_DATA_DIRPATH_ON_CLIENT_CONTAINER = "/data/gateway/execution-data"


def get_used_ports():
    used_ports = {
        RPC_PORT_ID: ethereum_package_shared_utils.new_port_spec(
            RPC_PORT_NUM,
            ethereum_package_shared_utils.TCP_PROTOCOL,
            ethereum_package_shared_utils.HTTP_APPLICATION_PROTOCOL,
        ),
        # ENGINE_RPC_PORT_ID: ethereum_package_shared_utils.new_port_spec(
        #     ENGINE_RPC_PORT_NUM, ethereum_package_shared_utils.TCP_PROTOCOL
        # ),
    }
    return used_ports


def launch(
    plan,
    launcher,
    service_name,
    service_image,
    node_selectors,
    existing_el_clients,
    sequencer_context,
    observability_helper,
):
    config = get_config(
        plan,
        launcher,
        service_name,
        service_image,
        node_selectors,
        existing_el_clients,
        sequencer_context,
        observability_helper,
    )

    service = plan.add_service(service_name, config)

    http_url = "http://{0}:{1}".format(service.ip_address, RPC_PORT_NUM)

    # metrics_info = observability.new_metrics_info(
    #     observability_helper, service, METRICS_PATH
    # )

    return ethereum_package_el_context.new_el_context(
        client_name="gateway",
        enode="",
        ip_addr=service.ip_address,
        rpc_port_num=RPC_PORT_NUM,
        ws_port_num=0,
        engine_rpc_port_num=RPC_PORT_NUM,
        rpc_http_url=http_url,
        service_name=service_name,
        #el_metrics_info=[metrics_info],
    )


def get_config(
    plan,
    launcher,
    service_name,
    service_image,
    global_log_level,
    node_selectors,
    existing_el_clients,
    sequencer_context,
):
    plan.print("Launching gateway: {0}".format(sequencer_context))

    ports = dict(get_used_ports())

    cmd = [
        "--chain={0}".format(
            launcher.network
            if launcher.network in ethereum_package_constants.PUBLIC_NETWORKS
            else ethereum_package_constants.GENESIS_CONFIG_MOUNT_PATH_ON_CONTAINER
            + "/genesis-{0}.json".format(launcher.network_id)
        ),
        "--db.datadir=" + EXECUTION_DATA_DIRPATH_ON_CLIENT_CONTAINER,
        "--rpc.fallback_url=" + sequencer_context.rpc_http_url,
        "--rpc.port={0}".format(RPC_PORT_NUM),
        "--gossip.root_peer_url=" + "http://fix.me", # TODO
        "--debug"
    ]

    # configure files

    files = {
        ethereum_package_constants.GENESIS_DATA_MOUNTPOINT_ON_CLIENTS: launcher.deployment_output,
        # ethereum_package_constants.JWT_MOUNTPOINT_ON_CLIENTS: launcher.jwt_file,
    }


    # apply customizations

    # if observability_helper.enabled:
    #     cmd.append("--metrics=0.0.0.0:{0}".format(observability.METRICS_PORT_NUM))

    #     observability.expose_metrics_port(ports)


    config_args = {
        "image": service_image,
        "ports": ports,
        "cmd": cmd,
        "files": files,
        "private_ip_address_placeholder": ethereum_package_constants.PRIVATE_IP_ADDRESS_PLACEHOLDER,
        "labels": ethereum_package_shared_utils.label_maker(
            client=constants.GATEWAY_TYPE.gateway,
            client_type=constants.CLIENT_TYPES.el,
            image=util.label_from_image(service_image),
            connected_client="",
            extra_labels={},
        ),
        "node_selectors": node_selectors,
    }


    return ServiceConfig(**config_args)


def new_gateway_launcher(
    deployment_output,
    jwt_file,
    network,
    network_id,
):
    return struct(
        deployment_output=deployment_output,
        jwt_file=jwt_file,
        network=network,
        network_id=network_id,
    )
