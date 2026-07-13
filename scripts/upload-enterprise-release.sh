#!/usr/bin/env bash

set -euo pipefail

SERVER="${GIT_AI_RELEASE_SERVER:-http://117.147.213.234:38080}"
VERSION="${GIT_AI_RELEASE_VERSION:-1.3.2}"
CHANNEL="${GIT_AI_RELEASE_CHANNEL:-latest}"
ASSET_DIR="${GIT_AI_RELEASE_ASSET_DIR:-$HOME/Downloads/git-ai release}"
ADMIN_KEY_FILE="${GIT_AI_ADMIN_KEY_FILE:-/tmp/git-ai-admin-key}"
CREDENTIAL_FILE="${GIT_AI_CREDENTIAL_FILE:-$HOME/.git-ai/internal/credentials}"

FILES=(
  git-ai-linux-x64
  git-ai-linux-arm64
  git-ai-windows-x64.exe
  git-ai-windows-arm64.exe
  git-ai-macos-x64
  git-ai-macos-arm64
)

if [[ -f "$CREDENTIAL_FILE" ]]; then
  access_token="$(jq -r '.access_token // empty' "$CREDENTIAL_FILE")"
  auth_header="Authorization: Bearer $access_token"
  credential_auth=true
elif [[ -f "$ADMIN_KEY_FILE" ]]; then
  admin_key="$(<"$ADMIN_KEY_FILE")"
  auth_header="X-API-Key: $admin_key"
else
  echo "No admin API key or CLI credential file found" >&2
  exit 2
fi

if [[ "$auth_header" == *": " ]]; then
  echo "Authentication credential is empty" >&2
  exit 2
fi

for filename in "${FILES[@]}"; do
  if [[ ! -f "$ASSET_DIR/$filename" ]]; then
    echo "Missing release asset: $ASSET_DIR/$filename" >&2
    exit 2
  fi
done

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT
manifest="$work_dir/SHA256SUMS"

for filename in "${FILES[@]}"; do
  hash="$(shasum -a 256 "$ASSET_DIR/$filename" | awk '{print $1}')"
  printf '%s  %s\n' "$hash" "$filename" >> "$manifest"
done

echo "Checking admin access at $SERVER"
auth_status="$(curl --silent --output /dev/null --write-out '%{http_code}' \
  -H "$auth_header" "$SERVER/api/admin/releases/assets")"

if [[ "$auth_status" == "401" && "${credential_auth:-false}" == "true" ]]; then
  echo "Refreshing expired or invalid CLI access token"
  refresh_token="$(jq -r '.refresh_token // empty' "$CREDENTIAL_FILE")"
  refresh_response="$(jq -n \
    --arg refresh_token "$refresh_token" \
    '{grant_type:"refresh_token",refresh_token:$refresh_token,client_id:"git-ai-cli"}' | \
    curl --fail-with-body --silent --show-error \
      -H "Content-Type: application/json" \
      --data-binary @- \
      "$SERVER/worker/oauth/token")"

  access_token="$(jq -er '.access_token' <<<"$refresh_response")"
  new_refresh_token="$(jq -er '.refresh_token' <<<"$refresh_response")"
  access_expires_in="$(jq -er '.expires_in' <<<"$refresh_response")"
  refresh_expires_in="$(jq -er '.refresh_expires_in' <<<"$refresh_response")"
  now="$(date +%s)"
  credential_tmp="$(mktemp "${CREDENTIAL_FILE}.tmp.XXXXXX")"
  chmod 600 "$credential_tmp"
  jq -n \
    --arg access_token "$access_token" \
    --arg refresh_token "$new_refresh_token" \
    --argjson access_token_expires_at "$((now + access_expires_in))" \
    --argjson refresh_token_expires_at "$((now + refresh_expires_in))" \
    '{access_token:$access_token,refresh_token:$refresh_token,access_token_expires_at:$access_token_expires_at,refresh_token_expires_at:$refresh_token_expires_at}' \
    > "$credential_tmp"
  mv "$credential_tmp" "$CREDENTIAL_FILE"
  auth_header="Authorization: Bearer $access_token"
  auth_status="$(curl --silent --output /dev/null --write-out '%{http_code}' \
    -H "$auth_header" "$SERVER/api/admin/releases/assets")"
fi

if [[ "$auth_status" != "200" ]]; then
  echo "Admin access check failed with HTTP $auth_status" >&2
  exit 1
fi

for filename in "${FILES[@]}"; do
  hash="$(shasum -a 256 "$ASSET_DIR/$filename" | awk '{print $1}')"
  echo "Uploading $filename"
  curl --fail-with-body --silent --show-error \
    -H "$auth_header" \
    -F "channel=$CHANNEL" \
    -F "version=$VERSION" \
    -F "filename=$filename" \
    -F "sha256=$hash" \
    -F "file=@$ASSET_DIR/$filename" \
    "$SERVER/api/admin/releases/upload"
  echo
done

manifest_hash="$(shasum -a 256 "$manifest" | awk '{print $1}')"
echo "Uploading SHA256SUMS"
curl --fail-with-body --silent --show-error \
  -H "$auth_header" \
  -F "channel=$CHANNEL" \
  -F "version=$VERSION" \
  -F "filename=SHA256SUMS" \
  -F "sha256=$manifest_hash" \
  -F "file=@$manifest" \
  "$SERVER/api/admin/releases/upload"
echo

echo "Publishing $CHANNEL -> $VERSION"
curl --fail-with-body --silent --show-error \
  -X POST \
  -H "$auth_header" \
  -H "Content-Type: application/json" \
  --data "{\"channel\":\"$CHANNEL\",\"version\":\"$VERSION\",\"checksum\":\"$manifest_hash\"}" \
  "$SERVER/api/admin/releases/channel"
echo

echo "Verifying public release metadata"
curl --fail-with-body --silent --show-error "$SERVER/worker/releases"
echo
