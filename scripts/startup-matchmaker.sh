#!/usr/bin/env bash
set -euxo pipefail

apt-get update
apt-get install -y docker.io curl jq
systemctl enable --now docker

TOKEN="$(curl -s -H 'Metadata-Flavor: Google'   http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token   | jq -r .access_token)"

echo "${TOKEN}" | docker login -u oauth2accesstoken --password-stdin   "https://us-west1-docker.pkg.dev"

docker pull "us-west1-docker.pkg.dev/project-cca5da57-2070-420b-bcd/forge/forge-matchmaker:relay-v1"
docker rm -f forge-matchmaker || true

docker run -d   --name forge-matchmaker   --restart unless-stopped   --network host   -e MATCHMAKER_BIND="0.0.0.0:7000"   -e RELAY_BIND="0.0.0.0:7001"   -e RELAY_ADVERTISE="136.118.42.95:7001"   "us-west1-docker.pkg.dev/project-cca5da57-2070-420b-bcd/forge/forge-matchmaker:relay-v1"
