shared_utils = import_module(
    "github.com/ethpandaops/ethereum-package/src/shared_utils/shared_utils.star"
)
el_context = import_module(
    "github.com/ethpandaops/ethereum-package/src/el/el_context.star"
)
el_admin_node_info = import_module(
    "github.com/ethpandaops/ethereum-package/src/el/el_admin_node_info.star"
)
constants = import_module(
    "github.com/ethpandaops/ethereum-package/src/package_io/constants.star"
)
gateway_launcher = import_module("../gateway/gateway_launcher.star")

RPC_PORT_NUM = 8081
RPC_PORT_ID = "rpc"
DEFAULT_IMAGE = "TODO_publish_images"
CONFIG_FILE_PATH = "/etc/registry/config.json.tmpl"

REGISTRY_DIRPATH = "../../../static_files/registry/registry.json.tmpl"
REGISTRY_FILENAME = "registry.json"
REGISTRY_MOUNT_DIRPATH_ON_SERVICE = "/config"
REGISTRY_FILES_ARTIFACT_NAME = "registry"


def get_used_ports():
    used_ports = {
        RPC_PORT_ID: shared_utils.new_port_spec(
            RPC_PORT_NUM,
            shared_utils.TCP_PROTOCOL,
            shared_utils.HTTP_APPLICATION_PROTOCOL,
        ),
    }
    return used_ports


def launch(
    plan,
    launcher,
    service_name,
    image,
    existing_el_clients,
    sequencer_context,  # fallback op geth
    builder_context,  # gateway
):
    network_name = shared_utils.get_network_name(launcher.network)

    config = get_config(
        plan,
        launcher.el_cl_genesis_data,
        launcher.jwt_file,
        launcher.network,
        launcher.network_id,
        image,
        service_name,
        existing_el_clients,
        sequencer_context,
        builder_context,
    )

    service = plan.add_service(service_name, config)

    http_url = "http://{0}:{1}".format(service.ip_address, RPC_PORT_NUM)

    return el_context.new_el_context(
        client_name="based_registry",
        enode=None,
        ip_addr=service.ip_address,
        rpc_port_num=RPC_PORT_NUM,
        ws_port_num=0,
        engine_rpc_port_num=RPC_PORT_NUM,
        rpc_http_url=http_url,
        service_name=service_name,
        el_metrics_info=None,
    )


def get_config(
    plan,
    el_cl_genesis_data,
    jwt_file,
    network,
    network_id,
    image,
    service_name,
    existing_el_clients,
    sequencer_context,
    builder_context,
):
    BUILDER_EXECUTION_ENGINE_ENDPOINT = "http://{0}:{1}".format(
        builder_context.ip_addr,
        builder_context.engine_rpc_port_num,
    )

    L2_EXECUTION_ENDPOINT = "http://{0}:{1}".format(
        sequencer_context.ip_addr,
        sequencer_context.rpc_port_num,
    )

    used_ports = get_used_ports()

    template_data = new_config_template_data(
        BUILDER_EXECUTION_ENGINE_ENDPOINT,
        gateway_launcher.GATEWAY_SIGNER_ADDRESS,
        gateway_launcher.GATEWAY_JWT,
    )

    registry_template = read_file(REGISTRY_DIRPATH)

    template_and_data = shared_utils.new_template_and_data(
        registry_template, template_data
    )

    template_and_data_by_rel_dest_filepath = {}
    template_and_data_by_rel_dest_filepath[REGISTRY_FILENAME] = template_and_data

    config_files_artifact_name = plan.render_templates(
        template_and_data_by_rel_dest_filepath,
        REGISTRY_FILES_ARTIFACT_NAME + "-" + service_name,
    )

    config_file_path = shared_utils.path_join(
        REGISTRY_MOUNT_DIRPATH_ON_SERVICE, REGISTRY_FILENAME
    )

    public_ports = {}
    cmd = [
        "--registry.path=" + config_file_path,
        "--eth_client.url={0}".format(L2_EXECUTION_ENDPOINT),
        "--debug",
    ]

    files = {
        REGISTRY_MOUNT_DIRPATH_ON_SERVICE: config_files_artifact_name,
    }

    return ServiceConfig(
        image=image,
        ports=used_ports,
        public_ports=public_ports,
        cmd=cmd,
        files=files,
        private_ip_address_placeholder=constants.PRIVATE_IP_ADDRESS_PLACEHOLDER,
    )


def new_registry_launcher(
    el_cl_genesis_data,
    jwt_file,
    network,
    network_id,
):
    return struct(
        el_cl_genesis_data=el_cl_genesis_data,
        jwt_file=jwt_file,
        network=network,
        network_id=network_id,
    )


def new_config_template_data(url, address, jwt):
    return {
        "Gateways": [
            {
                "URL": url,
                "Address": address,
                "JWT": jwt,
            }
        ],
    }
