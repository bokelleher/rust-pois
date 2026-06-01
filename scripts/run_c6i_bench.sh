#!/usr/bin/env bash
#
# run_c6i_bench.sh — Launch an AWS c6i.xlarge, run the SESAME (SCTE 130-9)
# Criterion benchmark on it, print the Markdown overhead table, and tear the
# instance down. This reproduces the paper §9.2 numbers on the hardware the
# paper cites (Intel Ice Lake, AES-NI + SHA extensions). Closes [BO-9].
#
# Connection is via SSM Session Manager (no inbound SSH, no key pair). The box
# only needs OUTBOUND internet (to reach rustup + crates.io + GitHub).
#
# ---------------------------------------------------------------------------
# PREREQUISITES
#   1. AWS CLI v2 configured (`aws configure`) with the IAM policy from
#      docs/ (ec2:RunInstances/TerminateInstances/Describe*, ssm:StartSession +
#      SendCommand/GetCommandInvocation, ssm:GetParameters, iam:PassRole).
#   2. An instance profile whose role has `AmazonSSMManagedInstanceCore`
#      attached — pass its NAME via --instance-profile.
#   3. The SESAME branch must be PUSHED to a Git remote the box can clone
#      (default repo/main). Pass --repo-url / --git-ref to point at your branch.
#      NOTE: as of this writing the SESAME code is uncommitted locally; push it
#      first, or the clone will lack benches/sesame_overhead.rs.
#   4. A vCPU quota >= 4 for On-Demand Standard instances in the region.
#
# Cost: c6i.xlarge ~ $0.17/hr on-demand; a run is a few minutes (a few cents).
# ---------------------------------------------------------------------------
set -euo pipefail

# ---- defaults ----
REGION="${AWS_REGION:-us-east-1}"
INSTANCE_TYPE="c6i.xlarge"
INSTANCE_PROFILE=""
SUBNET_ID=""
REPO_URL="https://github.com/bokelleher/rust-pois.git"
GIT_REF="main"
S3_BUCKET=""           # optional: capture full bench log to s3://<bucket>/...
KEEP=0                 # 1 = leave the instance running (debug)
SSM_AMI_PARAM="/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id"

usage() {
  cat <<EOF
Launch a c6i.xlarge, run the SESAME (SCTE 130-9) Criterion benchmark on it via
SSM, print the Markdown overhead table, and terminate. See the header of this
file for full prerequisites (IAM policy, SSM instance profile, pushed branch).

Usage: $0 --instance-profile <name> [options]

  --instance-profile NAME  (required) instance profile with AmazonSSMManagedInstanceCore
  --region REGION          AWS region                     (default: $REGION)
  --instance-type TYPE     EC2 type                        (default: $INSTANCE_TYPE)
  --subnet-id ID           subnet to launch in    (default: a default-VPC subnet)
  --repo-url URL           git repo to clone               (default: $REPO_URL)
  --git-ref REF            branch/tag/sha to bench         (default: $GIT_REF)
  --s3-bucket BUCKET       also stream full bench log to this bucket (optional)
  --keep                   do NOT terminate the instance afterward
  -h, --help               show this help
EOF
}

# ---- parse args ----
while [[ $# -gt 0 ]]; do
  case "$1" in
    --instance-profile) INSTANCE_PROFILE="$2"; shift 2 ;;
    --region)           REGION="$2"; shift 2 ;;
    --instance-type)    INSTANCE_TYPE="$2"; shift 2 ;;
    --subnet-id)        SUBNET_ID="$2"; shift 2 ;;
    --repo-url)         REPO_URL="$2"; shift 2 ;;
    --git-ref)          GIT_REF="$2"; shift 2 ;;
    --s3-bucket)        S3_BUCKET="$2"; shift 2 ;;
    --keep)             KEEP=1; shift ;;
    -h|--help)          usage; exit 0 ;;
    *) echo "unknown arg: $1" >&2; usage; exit 1 ;;
  esac
done

[[ -n "$INSTANCE_PROFILE" ]] || { echo "ERROR: --instance-profile is required" >&2; usage; exit 1; }
command -v aws >/dev/null || { echo "ERROR: aws CLI not found" >&2; exit 1; }

AWS=(aws --region "$REGION")
log() { echo "[$(date +%H:%M:%S)] $*" >&2; }

# ---- resolve AMI (Ubuntu 24.04 amd64, Canonical-published) ----
log "Resolving Ubuntu 24.04 AMI..."
AMI_ID=$("${AWS[@]}" ssm get-parameters --names "$SSM_AMI_PARAM" \
  --query 'Parameters[0].Value' --output text)
[[ "$AMI_ID" == ami-* ]] || { echo "ERROR: could not resolve AMI ($AMI_ID)" >&2; exit 1; }
log "AMI: $AMI_ID"

# ---- pick a default subnet if none supplied ----
if [[ -z "$SUBNET_ID" ]]; then
  SUBNET_ID=$("${AWS[@]}" ec2 describe-subnets \
    --filters Name=default-for-az,Values=true \
    --query 'Subnets[0].SubnetId' --output text)
  [[ "$SUBNET_ID" == subnet-* ]] || { echo "ERROR: no default subnet; pass --subnet-id" >&2; exit 1; }
  log "Subnet: $SUBNET_ID (default VPC)"
fi

# ---- remote script (runs as root via SSM) ----
# base64-encoded to dodge all shell quoting in the SSM parameter.
read -r -d '' REMOTE <<REMOTE_EOF || true
set -e
export DEBIAN_FRONTEND=noninteractive HOME=/root \\
  CARGO_HOME=/root/.cargo RUSTUP_HOME=/root/.rustup CARGO_TARGET_DIR=/root/target
apt-get update -qq
apt-get install -y -qq git build-essential python3 curl ca-certificates >/dev/null
curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable >/dev/null
. "\$CARGO_HOME/env"
cd /root
rm -rf rust-pois
git clone --depth 1 --branch "$GIT_REF" "$REPO_URL" rust-pois >/dev/null 2>&1
cd rust-pois
if [ ! -f benches/sesame_overhead.rs ]; then
  echo "ERROR: $REPO_URL@$GIT_REF has no benches/sesame_overhead.rs — push the SESAME branch first." >&2
  exit 3
fi
cargo bench --bench sesame_overhead >/root/bench.log 2>&1
echo '=====SESAME-BENCH-RESULT-BEGIN====='
echo "### Benchmark environment"
echo "- host: \$(curl -s http://169.254.169.254/latest/meta-data/instance-type 2>/dev/null || echo $INSTANCE_TYPE) (\$(nproc) vCPU)"
echo "- CPU: \$(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | sed 's/^ *//')"
echo "- AES-NI: \$(grep -qm1 ' aes' /proc/cpuinfo && echo yes || echo no); SHA ext: \$(grep -qm1 sha_ni /proc/cpuinfo && echo yes || echo no)"
echo "- OS: \$(. /etc/os-release; echo \$PRETTY_NAME), kernel \$(uname -r)"
echo "- toolchain: \$(rustc --version)"
echo "- repo: $REPO_URL @ \$(git rev-parse --short HEAD)"
echo
python3 scripts/bench_to_md.py /root/target/criterion
echo '=====SESAME-BENCH-RESULT-END====='
REMOTE_EOF

REMOTE_B64=$(printf '%s' "$REMOTE" | base64 -w0 2>/dev/null || printf '%s' "$REMOTE" | base64)
PARAM_FILE=$(mktemp)
RUNNER="echo $REMOTE_B64 | base64 -d | bash"
python3 - "$RUNNER" >"$PARAM_FILE" <<'PYJSON'
import json, sys
print(json.dumps({"commands": [sys.argv[1]]}))
PYJSON

# ---- launch ----
log "Launching $INSTANCE_TYPE ..."
INSTANCE_ID=$("${AWS[@]}" ec2 run-instances \
  --image-id "$AMI_ID" \
  --instance-type "$INSTANCE_TYPE" \
  --subnet-id "$SUBNET_ID" \
  --iam-instance-profile "Name=$INSTANCE_PROFILE" \
  --metadata-options 'HttpTokens=required,HttpEndpoint=enabled' \
  --instance-initiated-shutdown-behavior terminate \
  --block-device-mappings 'DeviceName=/dev/sda1,Ebs={VolumeSize=16,VolumeType=gp3,DeleteOnTermination=true}' \
  --tag-specifications 'ResourceType=instance,Tags=[{Key=Name,Value=sesame-bench}]' \
  --query 'Instances[0].InstanceId' --output text)
log "Instance: $INSTANCE_ID"

cleanup() {
  rm -f "$PARAM_FILE"
  if [[ "$KEEP" -eq 1 ]]; then
    log "--keep set; leaving $INSTANCE_ID running. Terminate with: aws --region $REGION ec2 terminate-instances --instance-ids $INSTANCE_ID"
  else
    log "Terminating $INSTANCE_ID ..."
    "${AWS[@]}" ec2 terminate-instances --instance-ids "$INSTANCE_ID" >/dev/null || true
  fi
}
trap cleanup EXIT

# ---- wait for running + SSM online ----
log "Waiting for instance to run..."
"${AWS[@]}" ec2 wait instance-running --instance-ids "$INSTANCE_ID"

log "Waiting for SSM agent to come online (can take 1-2 min)..."
for _ in $(seq 1 60); do
  PING=$("${AWS[@]}" ssm describe-instance-information \
    --filters "Key=InstanceIds,Values=$INSTANCE_ID" \
    --query 'InstanceInformationList[0].PingStatus' --output text 2>/dev/null || echo None)
  [[ "$PING" == "Online" ]] && break
  sleep 5
done
[[ "${PING:-}" == "Online" ]] || { echo "ERROR: SSM never came online" >&2; exit 2; }
log "SSM online."

# ---- run the benchmark via SSM ----
S3_ARGS=()
[[ -n "$S3_BUCKET" ]] && S3_ARGS=(--output-s3-bucket-name "$S3_BUCKET" --output-s3-key-prefix "sesame-bench/$INSTANCE_ID")

log "Sending benchmark command (installs Rust, clones, benches — ~3-6 min)..."
CMD_ID=$("${AWS[@]}" ssm send-command \
  --instance-ids "$INSTANCE_ID" \
  --document-name AWS-RunShellScript \
  --comment "SESAME SCTE 130-9 overhead benchmark" \
  --timeout-seconds 1800 \
  --parameters "file://$PARAM_FILE" \
  "${S3_ARGS[@]}" \
  --query 'Command.CommandId' --output text)
log "Command: $CMD_ID"

# ---- poll for completion ----
for _ in $(seq 1 120); do
  STATUS=$("${AWS[@]}" ssm get-command-invocation \
    --command-id "$CMD_ID" --instance-id "$INSTANCE_ID" \
    --query 'Status' --output text 2>/dev/null || echo Pending)
  case "$STATUS" in
    Success|Failed|Cancelled|TimedOut) break ;;
  esac
  sleep 15
done
log "Command status: ${STATUS:-unknown}"

OUT=$("${AWS[@]}" ssm get-command-invocation \
  --command-id "$CMD_ID" --instance-id "$INSTANCE_ID" \
  --query 'StandardOutputContent' --output text 2>/dev/null || true)
ERR=$("${AWS[@]}" ssm get-command-invocation \
  --command-id "$CMD_ID" --instance-id "$INSTANCE_ID" \
  --query 'StandardErrorContent' --output text 2>/dev/null || true)

echo
echo "================ SESAME benchmark result ($INSTANCE_TYPE, $REGION) ================"
if echo "$OUT" | grep -q 'SESAME-BENCH-RESULT-BEGIN'; then
  echo "$OUT" | sed -n '/SESAME-BENCH-RESULT-BEGIN/,/SESAME-BENCH-RESULT-END/p' \
                | grep -v 'SESAME-BENCH-RESULT-'
else
  echo "$OUT"
  echo "---- stderr ----" >&2
  echo "$ERR" >&2
fi
echo "==============================================================================="
[[ -n "$S3_BUCKET" ]] && log "Full bench log: s3://$S3_BUCKET/sesame-bench/$INSTANCE_ID/"

[[ "$STATUS" == "Success" ]] || { echo "Benchmark command did not succeed (status: $STATUS)." >&2; exit 1; }
log "Done. Paste the tables above into docs/benchmarks.md and the paper §9.2."
