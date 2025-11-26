# 🏗️ Build modified Rabby wallet for Google Chrome and Firefox
build-rabby-chrome:
    cd rabby && yarn && \
      yarn build:pro && \
      yarn build:pro:mv2
