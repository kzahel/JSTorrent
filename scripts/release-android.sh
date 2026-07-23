#!/usr/bin/env bash
set -e

VERSION="$1"

if [ -z "$VERSION" ]; then
  echo "Usage: $0 <version>"
  exit 1
fi

if [[ ! "$VERSION" =~ ^[0-9] ]]; then
  echo "Error: Version must start with a number (e.g., 1.0.0, not v1.0.0)"
  exit 1
fi

# Fail if working tree is dirty (avoid releasing with uncommitted changes)
if ! git diff-index --quiet HEAD --; then
  echo "Error: Working tree has uncommitted changes. Please commit or stash first."
  git diff --stat
  exit 1
fi

TAG="android-v${VERSION}"
BUILD_GRADLE="android/app/build.gradle.kts"
CHANGELOG="android/CHANGELOG.md"

# Check that changelog has been updated (hard fail)
if ! grep -q "## \[${VERSION}\]" "$CHANGELOG" 2>/dev/null; then
  echo "Error: $CHANGELOG doesn't have an entry for version ${VERSION}"
  echo "Please add a '## [${VERSION}]' section before releasing."
  exit 1
fi

# Get current versionCode and increment
CURRENT_CODE=$(grep -o 'versionCode = [0-9]*' "$BUILD_GRADLE" | grep -o '[0-9]*')
NEW_CODE=$((CURRENT_CODE + 1))

echo "Updating version: $CURRENT_CODE -> $NEW_CODE, versionName: $VERSION"

# Update build.gradle.kts (GNU sed on Linux, BSD sed on macOS)
if sed --version >/dev/null 2>&1; then
  sed -i "s/versionCode = $CURRENT_CODE/versionCode = $NEW_CODE/" "$BUILD_GRADLE"
  sed -i "s/versionName = \"[^\"]*\"/versionName = \"$VERSION\"/" "$BUILD_GRADLE"
else
  sed -i '' "s/versionCode = $CURRENT_CODE/versionCode = $NEW_CODE/" "$BUILD_GRADLE"
  sed -i '' "s/versionName = \"[^\"]*\"/versionName = \"$VERSION\"/" "$BUILD_GRADLE"
fi

# Commit, tag, and push
git add "$BUILD_GRADLE" "$CHANGELOG"
git commit -m "Release Android v${VERSION}"
git tag "$TAG"
git push origin main "$TAG"

echo "Released Android v${VERSION} (versionCode: $NEW_CODE)"
echo "CI will build the signed APK: https://github.com/kzahel/jstorrent/actions"
