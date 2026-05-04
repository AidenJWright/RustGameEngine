# Google Cloud Matchmaker Relay Deployment

This deployment runs the matchmaker and gameplay packet relay in one container on
one Compute Engine VM. The matchmaker still does not simulate the game. It only:

- creates and starts lobbies on UDP `7000`
- assigns each player a per-match relay token
- relays `Input`, `HostHash`, and `HostCorrection` packets on UDP `7001`
- hides all player IP addresses from other players

End users only need outbound UDP access to the server. They do not need router
port forwarding, UPnP, NAT-PMP, STUN, or TURN.

### CURRENT SERVER ADDRESS
Server IP: 136.118.42.95

## 1. Choose Deployment Values

```bash
export PROJECT_ID="project-cca5da57-2070-420b-bcd"
export REGION="us-west1"
export ZONE="us-west1-a"
export REPO="forge"
export IMAGE_NAME="forge-matchmaker"
export IMAGE_TAG="relay-v1"
export VM_NAME="forge-matchmaker-1"
export NETWORK_TAG="forge-matchmaker"
export ADDRESS_NAME="forge-matchmaker-ip"
export SERVICE_ACCOUNT_NAME="forge-matchmaker-vm"

export IMAGE_URI="${REGION}-docker.pkg.dev/${PROJECT_ID}/${REPO}/${IMAGE_NAME}:${IMAGE_TAG}"
```

Use a region close to most players. For classmates in California, `us-west1` is
a reasonable first region.

## 2. Enable Google Cloud APIs

```bash
gcloud config set project "${PROJECT_ID}"

gcloud services enable \
  compute.googleapis.com \
  artifactregistry.googleapis.com \
  logging.googleapis.com \
  monitoring.googleapis.com
```

## 3. Build and Push the Container

```bash
gcloud artifacts repositories create "${REPO}" \
  --repository-format=docker \
  --location="${REGION}" \
  --description="Forge ECS game server images"

gcloud auth configure-docker "${REGION}-docker.pkg.dev"

docker build \
  -f Dockerfile.matchmaker \
  -t "${IMAGE_URI}" \
  .

docker push "${IMAGE_URI}"
```

## 4. Reserve a Static External IP

```bash
gcloud compute addresses create "${ADDRESS_NAME}" \
  --region="${REGION}"

export SERVER_IP="$(gcloud compute addresses describe "${ADDRESS_NAME}" \
  --region="${REGION}" \
  --format='value(address)')"

echo "Server IP: ${SERVER_IP}"
```


Server IP: 136.118.42.95



Players will enter `${SERVER_IP}:7000` as the matchmaker address. The matchmaker
will advertise `${SERVER_IP}:7001` as the gameplay relay.

## 5. Create the VM Service Account

```bash
gcloud iam service-accounts create "${SERVICE_ACCOUNT_NAME}" \
  --display-name="Forge matchmaker VM"

export SERVICE_ACCOUNT_EMAIL="${SERVICE_ACCOUNT_NAME}@${PROJECT_ID}.iam.gserviceaccount.com"

gcloud projects add-iam-policy-binding "${PROJECT_ID}" \
  --member="serviceAccount:${SERVICE_ACCOUNT_EMAIL}" \
  --role="roles/artifactregistry.reader"

gcloud projects add-iam-policy-binding "${PROJECT_ID}" \
  --member="serviceAccount:${SERVICE_ACCOUNT_EMAIL}" \
  --role="roles/logging.logWriter"
```

## 6. Open Only the Required UDP Ports

```bash
gcloud compute firewall-rules create forge-matchmaker-control-udp \
  --network=default \
  --direction=INGRESS \
  --priority=1000 \
  --target-tags="${NETWORK_TAG}" \
  --source-ranges=0.0.0.0/0 \
  --allow=udp:7000

gcloud compute firewall-rules create forge-matchmaker-relay-udp \
  --network=default \
  --direction=INGRESS \
  --priority=1000 \
  --target-tags="${NETWORK_TAG}" \
  --source-ranges=0.0.0.0/0 \
  --allow=udp:7001
```

## 7. Create the VM Startup Script

```bash
cat > scripts/startup-matchmaker.sh <<EOF
#!/usr/bin/env bash
set -euxo pipefail

apt-get update
apt-get install -y docker.io curl jq
systemctl enable --now docker

TOKEN="\$(curl -s -H 'Metadata-Flavor: Google' \
  http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token \
  | jq -r .access_token)"

echo "\${TOKEN}" | docker login -u oauth2accesstoken --password-stdin \
  "https://${REGION}-docker.pkg.dev"

docker pull "${IMAGE_URI}"
docker rm -f forge-matchmaker || true

docker run -d \
  --name forge-matchmaker \
  --restart unless-stopped \
  --network host \
  -e MATCHMAKER_BIND="0.0.0.0:7000" \
  -e RELAY_BIND="0.0.0.0:7001" \
  -e RELAY_ADVERTISE="${SERVER_IP}:7001" \
  "${IMAGE_URI}"
EOF
```

## 8. Create the Compute Engine VM

```bash
gcloud compute instances create "${VM_NAME}" \
  --zone="${ZONE}" \
  --machine-type=e2-medium \
  --image-family=debian-12 \
  --image-project=debian-cloud \
  --address="${SERVER_IP}" \
  --tags="${NETWORK_TAG}" \
  --service-account="${SERVICE_ACCOUNT_EMAIL}" \
  --scopes=https://www.googleapis.com/auth/cloud-platform \
  --metadata-from-file startup-script=scripts/startup-matchmaker.sh
```

## 9. Verify the Server

Check the container logs:

```bash
gcloud compute ssh "${VM_NAME}" --zone="${ZONE}" \
  --command='sudo docker logs --tail=100 forge-matchmaker'
```

Expected log shape:

```text
Matchmaker listening on 0.0.0.0:7000
Gameplay relay listening on 0.0.0.0:7001, advertising <SERVER_IP>:7001
```

From your dev machine:

```bash
cargo run --bin matchmaker_client -- \
  --server "${SERVER_IP}:7000" \
  ping
```

Then launch two game clients and connect to:

```text
<SERVER_IP>:7000
```

No client should be asked for a local gameplay port. The `MatchStart` event
contains only player identities and that player's relay token.

## 10. Update or Roll Back

Build and push a new image tag:

```bash
export IMAGE_TAG="relay-v7"
export IMAGE_URI="${REGION}-docker.pkg.dev/${PROJECT_ID}/${REPO}/${IMAGE_NAME}:${IMAGE_TAG}"

docker build -f Dockerfile.matchmaker -t "${IMAGE_URI}" .
docker push "${IMAGE_URI}"
```

SSH to the VM and restart with the new image:

```bash
gcloud compute ssh "${VM_NAME}" --zone="${ZONE}" --command="
TOKEN=\$(curl -s -H 'Metadata-Flavor: Google' \
  http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token \
  | jq -r .access_token) &&
echo \"\${TOKEN}\" | sudo docker login -u oauth2accesstoken --password-stdin \
  https://${REGION}-docker.pkg.dev &&
sudo docker pull '${IMAGE_URI}' &&
sudo docker rm -f forge-matchmaker &&
sudo docker run -d \
  --name forge-matchmaker \
  --restart unless-stopped \
  --network host \
  -e MATCHMAKER_BIND='0.0.0.0:7000' \
  -e RELAY_BIND='0.0.0.0:7001' \
  -e RELAY_ADVERTISE='${SERVER_IP}:7001' \
  '${IMAGE_URI}'
"
```

For rollback, run the same command with the previous image tag.

## 11. Production Hardening Checklist

- Add a DNS `A` record, for example `matchmaker.example.com -> ${SERVER_IP}`.
- Set uptime checks for UDP reachability with a custom probe or a small health
  client running in Cloud Monitoring.
- Add alerts for VM CPU, network egress, and process restarts.
- Use one VM per region when player geography grows.
- Add graceful drain before updates: stop sending new matches to a VM, wait for
  active relay matches to finish, then restart the container.
- Keep packet sizes below roughly 1200 bytes where practical to avoid UDP
  fragmentation.

## Reference Docs

- Artifact Registry Docker push/pull:
  https://cloud.google.com/artifact-registry/docs/docker/pushing-and-pulling
- Static external IP addresses:
  https://cloud.google.com/compute/docs/ip-addresses/reserve-static-external-ip-address
- VPC firewall rules:
  https://cloud.google.com/firewall/docs/firewalls
- Compute Engine VM creation:
  https://cloud.google.com/compute/docs/instances/create-vm-specific-subnet
