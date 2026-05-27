#!/bin/bash
# Upload JRE tarballs and manifest to GCS for native mode deployments.
# Run this once to populate gs://puru-releases/jres/
#
# Prerequisites:
#   - gcloud CLI authenticated with access to puru-255206
#   - Download JRE tarballs from https://adoptium.net/temurin/releases/

set -e

BUCKET="gs://puru-releases"

echo "=== Puru Nucleus — JRE Upload ==="
echo ""

# Check if JRE tarballs exist in current directory
if ls temurin-*.tar.gz 1>/dev/null 2>&1; then
    echo "Found JRE tarballs:"
    ls -lh temurin-*.tar.gz
    echo ""
else
    echo "No JRE tarballs found. Download them first:"
    echo ""
    echo "  Java 21 (Windows x64):"
    echo "    https://adoptium.net/temurin/releases/?version=21&os=windows&arch=x64&package=jre"
    echo "    Rename to: temurin-21-windows-x64.tar.gz"
    echo ""
    echo "  Java 21 (Linux x64):"
    echo "    https://adoptium.net/temurin/releases/?version=21&os=linux&arch=x64&package=jre"
    echo "    Rename to: temurin-21-linux-x64.tar.gz"
    echo ""
    echo "  Java 25 (Windows x64) — if needed for puru-auth/mercury:"
    echo "    Rename to: temurin-25-windows-x64.tar.gz"
    echo ""
    exit 1
fi

# Upload each tarball
for tarball in temurin-*.tar.gz; do
    echo "Uploading $tarball ..."
    gsutil cp "$tarball" "$BUCKET/jres/$tarball"
done

# Generate and upload jre-manifest.json
echo ""
echo "Generating jre-manifest.json ..."

cat > jre-manifest.json << 'MANIFEST'
{
  "21": {
    "windows-x64": "temurin-21-windows-x64.tar.gz",
    "linux-x64": "temurin-21-linux-x64.tar.gz",
    "mac-x64": "temurin-21-mac-x64.tar.gz",
    "mac-arm64": "temurin-21-mac-arm64.tar.gz"
  },
  "25": {
    "windows-x64": "temurin-25-windows-x64.tar.gz",
    "linux-x64": "temurin-25-linux-x64.tar.gz",
    "mac-x64": "temurin-25-mac-x64.tar.gz",
    "mac-arm64": "temurin-25-mac-arm64.tar.gz"
  }
}
MANIFEST

gsutil cp jre-manifest.json "$BUCKET/jres/jre-manifest.json"

echo ""
echo "Done! Uploaded JRE tarballs + manifest to $BUCKET/jres/"
echo ""
echo "Verify with: gsutil ls $BUCKET/jres/"
