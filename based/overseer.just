import 'portal.just'
import 'scripts/consts.just'

start:
    docker exec -it based-gateway overseer --portal-url {{PORTAL}} --rich-wallet-key {{DUMMY_RICH_WALLET_PRIVATE_KEY}}
