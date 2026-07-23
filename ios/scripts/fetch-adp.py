#!/usr/bin/env python3
"""
Submit for notarization and fetch the Alternative Distribution Package (ADP).

This script handles the full post-upload flow:
  1. Find the app and latest build in ASC
  2. Wait for the build to finish processing
  3. Create an app store version and submit for notarization
  4. Wait for notarization to complete and ADP to be generated
  5. Download ADP variants
  6. Output the local ADP directory path

Prerequisites:
  - Build already uploaded to ASC via `xcrun altool --upload-app`
  - App configured for alternative distribution in ASC
    (AltStore PAL marketplace added, app selected)

Usage:
    uv run --project ios python ios/scripts/fetch-adp.py \\
        --key-path ~/private_keys/AuthKey_XXX.p8 \\
        --key-id XXX \\
        --issuer-id YYY \\
        --bundle-id com.jstorrent.ios \\
        --version 1.0.1 \\
        --output-dir ./adp
"""

import argparse
import json
import os
import sys
import time
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.error import HTTPError

try:
    import jwt
except ImportError:
    print(
        "ERROR: PyJWT not installed. Run: uv sync --project ios --locked",
        file=sys.stderr,
    )
    sys.exit(1)

ASC_BASE = "https://api.appstoreconnect.apple.com/v1"
POLL_INTERVAL = 30  # seconds
MAX_WAIT_PROCESSING = 600  # 10 min for build processing
MAX_WAIT_NOTARIZATION = 10800  # 3 hours for notarization (free macOS runners)


def generate_jwt(key_path: str, key_id: str, issuer_id: str) -> str:
    with open(key_path, "r") as f:
        private_key = f.read()
    now = int(time.time())
    payload = {
        "iss": issuer_id,
        "iat": now,
        "exp": now + 1200,  # 20 min max
        "aud": "appstoreconnect-v1",
    }
    return jwt.encode(
        payload,
        private_key,
        algorithm="ES256",
        headers={"kid": key_id, "typ": "JWT"},
    )


class ASCClient:
    def __init__(self, key_path: str, key_id: str, issuer_id: str):
        self.key_path = key_path
        self.key_id = key_id
        self.issuer_id = issuer_id
        self._token = None
        self._token_exp = 0

    @property
    def token(self) -> str:
        now = time.time()
        if self._token is None or now >= self._token_exp - 60:
            self._token = generate_jwt(self.key_path, self.key_id, self.issuer_id)
            self._token_exp = now + 1200
        return self._token

    def _request(self, method: str, path: str, params=None, body=None) -> dict:
        url = f"{ASC_BASE}{path}"
        if params:
            qs = "&".join(f"{k}={v}" for k, v in params.items())
            url = f"{url}?{qs}"
        headers = {
            "Authorization": f"Bearer {self.token}",
            "Content-Type": "application/json",
        }
        data = json.dumps(body).encode() if body else None
        req = Request(url, data=data, headers=headers, method=method)
        try:
            with urlopen(req) as resp:
                raw = resp.read()
                return json.loads(raw) if raw else {}
        except HTTPError as e:
            resp_body = e.read().decode() if e.fp else ""
            print(
                f"ASC API {method} {e.code} for {url}: {resp_body}", file=sys.stderr
            )
            # Attach body to exception so callers can inspect it
            e.response_body = resp_body
            raise

    def get(self, path: str, params=None) -> dict:
        return self._request("GET", path, params=params)

    def post(self, path: str, body: dict) -> dict:
        return self._request("POST", path, body=body)

    def patch(self, path: str, body: dict) -> dict:
        return self._request("PATCH", path, body=body)

    def delete(self, path: str):
        return self._request("DELETE", path)


def find_app(client: ASCClient, bundle_id: str) -> str:
    data = client.get(
        "/apps", {"filter[bundleId]": bundle_id, "fields[apps]": "bundleId,name"}
    )
    apps = data.get("data", [])
    if not apps:
        print(f"ERROR: No app found with bundle ID: {bundle_id}", file=sys.stderr)
        sys.exit(1)
    app_id = apps[0]["id"]
    name = apps[0].get("attributes", {}).get("name", "")
    print(f"Found app: {app_id} ({name})")
    return app_id


def find_latest_build(
    client: ASCClient, app_id: str, min_build_number: int = 0
) -> dict:
    """Find the latest build, optionally waiting for a build >= min_build_number."""
    max_wait = 600  # 10 minutes for build to appear after upload
    elapsed = 0
    while True:
        data = client.get(
            "/builds",
            {
                "filter[app]": app_id,
                "sort": "-uploadedDate",
                "limit": "1",
                "fields[builds]": "version,uploadedDate,processingState",
            },
        )
        builds = data.get("data", [])
        if not builds:
            if elapsed >= max_wait:
                print("ERROR: No builds found", file=sys.stderr)
                sys.exit(1)
        else:
            build = builds[0]
            attrs = build.get("attributes", {})
            build_version = int(attrs.get("version", "0"))
            print(
                f"Latest build: {build['id']} "
                f"(version={build_version}, "
                f"uploaded={attrs.get('uploadedDate')}, "
                f"state={attrs.get('processingState')})"
            )
            if build_version >= min_build_number:
                return build
            print(
                f"  Waiting for build >= {min_build_number} "
                f"(current: {build_version}, waited {elapsed}s)"
            )

        if elapsed >= max_wait:
            print(
                f"ERROR: Build >= {min_build_number} not found after {max_wait}s",
                file=sys.stderr,
            )
            sys.exit(1)
        time.sleep(POLL_INTERVAL)
        elapsed += POLL_INTERVAL


def wait_for_processing(client: ASCClient, build_id: str):
    """Wait for build to finish processing in ASC."""
    elapsed = 0
    while elapsed < MAX_WAIT_PROCESSING:
        data = client.get(
            f"/builds/{build_id}",
            {"fields[builds]": "processingState"},
        )
        state = data["data"]["attributes"]["processingState"]
        if state == "VALID":
            print("Build processing complete.")
            return
        if state == "INVALID":
            print("ERROR: Build is INVALID", file=sys.stderr)
            sys.exit(1)
        print(f"Build processing state: {state} (waited {elapsed}s)")
        time.sleep(POLL_INTERVAL)
        elapsed += POLL_INTERVAL
    print(
        f"ERROR: Build processing timed out after {MAX_WAIT_PROCESSING}s",
        file=sys.stderr,
    )
    sys.exit(1)


def _find_reusable_version(
    client: ASCClient, app_id: str, version_string: str
) -> str:
    """Find a non-terminal version that can be repurposed for the new release.

    ASC won't let you create a new version while another non-terminal version
    exists. This finds such a version, gets it into an editable state
    (cancelling active submissions if needed), and updates its version string.
    """
    REUSABLE_STATES = {
        "PREPARE_FOR_SUBMISSION",
        "DEVELOPER_REJECTED",
        "WAITING_FOR_REVIEW",
        "IN_REVIEW",
    }
    data = client.get(
        f"/apps/{app_id}/appStoreVersions",
        {
            "filter[platform]": "IOS",
            "fields[appStoreVersions]": "versionString,appVersionState",
        },
    )
    for v in data.get("data", []):
        attrs = v.get("attributes", {})
        vs = attrs.get("versionString", "")
        state = attrs.get("appVersionState", "")
        if vs == version_string:
            continue  # Already handled by find_or_create_version
        if state not in REUSABLE_STATES:
            continue

        print(f"Found reusable version {vs} ({v['id']}, state={state})")

        # If in review or waiting, cancel the submission first
        if state in ("WAITING_FOR_REVIEW", "IN_REVIEW"):
            print(f"  Cancelling active submissions for version {vs}...")
            cancel_conflicting_submissions(client, app_id, v["id"])
            # Wait for version to become editable
            for attempt in range(12):
                vdata = client.get(
                    f"/appStoreVersions/{v['id']}",
                    {"fields[appStoreVersions]": "appVersionState"},
                )
                new_state = vdata["data"]["attributes"]["appVersionState"]
                if new_state in ("PREPARE_FOR_SUBMISSION", "DEVELOPER_REJECTED"):
                    break
                print(f"  Version state: {new_state}, waiting... ({attempt * 5}s)")
                time.sleep(5)

        print(f"  Repurposing as {version_string}...")
        try:
            client.patch(
                f"/appStoreVersions/{v['id']}",
                {
                    "data": {
                        "type": "appStoreVersions",
                        "id": v["id"],
                        "attributes": {"versionString": version_string},
                    }
                },
            )
            print(f"  Updated version string to {version_string}")
            return v["id"]
        except HTTPError as e:
            print(f"  Could not repurpose: {e.code}")
            continue
    return None


def find_or_create_version(
    client: ASCClient, app_id: str, version_string: str, build_id: str
) -> str:
    """Find an existing appStoreVersion for this version string, or create one."""
    # Check for existing version with matching version string
    data = client.get(
        f"/apps/{app_id}/appStoreVersions",
        {
            "filter[platform]": "IOS",
            "filter[versionString]": version_string,
            "fields[appStoreVersions]": "versionString,appStoreState,reviewType",
        },
    )
    versions = data.get("data", [])
    for v in versions:
        state = v.get("attributes", {}).get("appStoreState", "")
        print(f"Found existing version {version_string}: {v['id']} (state={state})")
        return v["id"]

    # Try to repurpose a PREPARE_FOR_SUBMISSION version (can't create new
    # versions when one exists in this state, and can't delete them either)
    reused = _find_reusable_version(client, app_id, version_string)
    if reused:
        return reused

    # Create new version with reviewType NOTARIZATION
    print(f"Creating new app store version {version_string}...")
    body = {
        "data": {
            "type": "appStoreVersions",
            "attributes": {
                "platform": "IOS",
                "versionString": version_string,
                "reviewType": "NOTARIZATION",
            },
            "relationships": {
                "app": {"data": {"type": "apps", "id": app_id}},
                "build": {"data": {"type": "builds", "id": build_id}},
            },
        }
    }
    result = client.post("/appStoreVersions", body)
    version_id = result["data"]["id"]
    print(f"Created version: {version_id}")
    return version_id


def set_review_type_notarization(client: ASCClient, version_id: str):
    """Ensure the version's review type is set to NOTARIZATION."""
    print(f"Setting review type to NOTARIZATION for version {version_id}...")
    try:
        client.patch(
            f"/appStoreVersions/{version_id}",
            {
                "data": {
                    "type": "appStoreVersions",
                    "id": version_id,
                    "attributes": {"reviewType": "NOTARIZATION"},
                }
            },
        )
        print("Review type set to NOTARIZATION.")
    except HTTPError as e:
        if e.code == 409:
            print("Version already submitted or review type already set, continuing...")
        else:
            raise


def attach_build_to_version(client: ASCClient, version_id: str, build_id: str):
    """Attach the build to the version if not already attached."""
    print(f"Attaching build {build_id} to version {version_id}...")
    try:
        client.patch(
            f"/appStoreVersions/{version_id}",
            {
                "data": {
                    "type": "appStoreVersions",
                    "id": version_id,
                    "relationships": {
                        "build": {"data": {"type": "builds", "id": build_id}}
                    },
                }
            },
        )
        print("Build attached.")
    except HTTPError as e:
        if e.code == 409:
            print("Build already attached or version already submitted, continuing...")
        else:
            raise


def _cancel_submission(client: ASCClient, sub_id: str):
    """Cancel a single submission, handling various states."""
    try:
        client.patch(
            f"/reviewSubmissions/{sub_id}",
            {
                "data": {
                    "type": "reviewSubmissions",
                    "id": sub_id,
                    "attributes": {"canceled": True},
                }
            },
        )
        return True
    except HTTPError as e:
        if e.code == 409:
            return False
        raise


def cancel_conflicting_submissions(
    client: ASCClient, app_id: str, version_id: str
):
    """Cancel any non-terminal review submissions to free up concurrency slots."""
    # Get all non-terminal submissions
    non_terminal_states = (
        "READY_FOR_REVIEW,UNRESOLVED_ISSUES,WAITING_FOR_REVIEW,IN_REVIEW,CANCELING"
    )
    data = client.get(
        f"/apps/{app_id}/reviewSubmissions",
        {"filter[state]": non_terminal_states},
    )
    subs = data.get("data", [])
    if not subs:
        print("No conflicting submissions found.")
        return

    print(f"Found {len(subs)} non-terminal submission(s) to clean up...")

    for sub in subs:
        sub_id = sub["id"]
        state = sub["attributes"]["state"]

        if state == "CANCELING":
            print(f"  {sub_id}: already cancelling")
            continue

        if state == "READY_FOR_REVIEW":
            # READY_FOR_REVIEW can't be cancelled directly. We need to add an
            # item, submit it, wait for it to become WAITING_FOR_REVIEW, then
            # cancel. We need a version in a submittable state to do this.
            print(f"  {sub_id}: READY_FOR_REVIEW — need submit-then-cancel")
            # Wait for version to be submittable
            for attempt in range(12):
                v = client.get(
                    f"/appStoreVersions/{version_id}",
                    {"fields[appStoreVersions]": "appVersionState"},
                )
                vstate = v["data"]["attributes"]["appVersionState"]
                if vstate in ("PREPARE_FOR_SUBMISSION", "DEVELOPER_REJECTED"):
                    break
                print(f"    Version state: {vstate}, waiting... ({attempt * 5}s)")
                time.sleep(5)
            try:
                client.post(
                    "/reviewSubmissionItems",
                    {
                        "data": {
                            "type": "reviewSubmissionItems",
                            "relationships": {
                                "reviewSubmission": {
                                    "data": {
                                        "type": "reviewSubmissions",
                                        "id": sub_id,
                                    }
                                },
                                "appStoreVersion": {
                                    "data": {
                                        "type": "appStoreVersions",
                                        "id": version_id,
                                    }
                                },
                            },
                        }
                    },
                )
                client.patch(
                    f"/reviewSubmissions/{sub_id}",
                    {
                        "data": {
                            "type": "reviewSubmissions",
                            "id": sub_id,
                            "attributes": {"submitted": True},
                        }
                    },
                )
                time.sleep(3)
                if _cancel_submission(client, sub_id):
                    print(f"  {sub_id}: submitted and cancelled")
                else:
                    print(f"  {sub_id}: submitted but cancel failed")
                time.sleep(5)
            except HTTPError:
                print(f"  {sub_id}: could not clear (submit failed)")
            continue

        # WAITING_FOR_REVIEW, IN_REVIEW, UNRESOLVED_ISSUES — try direct cancel
        print(f"  {sub_id}: {state} — cancelling...")
        if not _cancel_submission(client, sub_id):
            print(f"  {sub_id}: could not cancel, may already be terminal")

    # Wait for all cancellations to complete
    max_wait = 300
    elapsed = 0
    while elapsed < max_wait:
        data = client.get(
            f"/apps/{app_id}/reviewSubmissions",
            {"filter[state]": "CANCELING"},
        )
        cancelling = data.get("data", [])
        if not cancelling:
            print("All conflicting submissions cleared.")
            return
        print(f"  Still cancelling {len(cancelling)} submission(s)... ({elapsed}s)")
        time.sleep(10)
        elapsed += 10
    print("WARNING: Cancellation still pending after 5 min, proceeding anyway...")


def _create_and_submit(
    client: ASCClient, app_id: str, version_id: str, submission_id: str = None
):
    """Create (or reuse) a review submission, add the version, and submit."""
    if not submission_id:
        result = client.post(
            "/reviewSubmissions",
            {
                "data": {
                    "type": "reviewSubmissions",
                    "attributes": {"platform": "IOS"},
                    "relationships": {
                        "app": {"data": {"type": "apps", "id": app_id}}
                    },
                }
            },
        )
        submission_id = result["data"]["id"]
        print(f"Created review submission: {submission_id}")

    # Add the version as a submission item
    client.post(
        "/reviewSubmissionItems",
        {
            "data": {
                "type": "reviewSubmissionItems",
                "relationships": {
                    "reviewSubmission": {
                        "data": {
                            "type": "reviewSubmissions",
                            "id": submission_id,
                        }
                    },
                    "appStoreVersion": {
                        "data": {
                            "type": "appStoreVersions",
                            "id": version_id,
                        }
                    },
                },
            }
        },
    )
    print("Added version to submission.")

    # Submit
    client.patch(
        f"/reviewSubmissions/{submission_id}",
        {
            "data": {
                "type": "reviewSubmissions",
                "id": submission_id,
                "attributes": {"submitted": True},
            }
        },
    )
    print("Submitted for notarization!")


def _check_already_submitted(client: ASCClient, app_id: str, version_id: str) -> bool:
    """Check if this version is already part of an active submission."""
    data = client.get(
        f"/apps/{app_id}/reviewSubmissions",
        {
            "filter[state]": "WAITING_FOR_REVIEW,IN_REVIEW",
        },
    )
    for sub in data.get("data", []):
        # Check if this submission contains our version
        items = client.get(
            f"/reviewSubmissions/{sub['id']}/items",
            {"fields[reviewSubmissionItems]": "state"},
        )
        for item in items.get("data", []):
            relationships = item.get("relationships", {})
            ver_data = relationships.get("appStoreVersion", {}).get("data", {})
            if ver_data.get("id") == version_id:
                print(
                    f"Version already in active submission {sub['id']} "
                    f"(state={sub['attributes']['state']})"
                )
                return True
    return False


def submit_for_notarization(client: ASCClient, app_id: str, version_id: str):
    """Submit the version for review/notarization."""
    print("Submitting for notarization...")

    # Check if already submitted
    if _check_already_submitted(client, app_id, version_id):
        return

    # Cancel stale submissions to free up concurrency slots
    cancel_conflicting_submissions(client, app_id, version_id)

    # Try the newer reviewSubmissions API first
    try:
        _create_and_submit(client, app_id, version_id)
        return
    except HTTPError as e:
        body = getattr(e, "response_body", "")
        if e.code == 409:
            if "ITEM_PART_OF_ANOTHER_SUBMISSION" in body:
                print(
                    "Version is claimed by another submission. "
                    "Cancelling conflicting submissions..."
                )
                cancel_conflicting_submissions(client, app_id, version_id)
                # Retry after cancellation
                try:
                    _create_and_submit(client, app_id, version_id)
                    return
                except HTTPError as e2:
                    if e2.code == 409:
                        print("Already submitted for review, continuing...")
                        return
                    raise
            elif "CONCURRENT_REVIEW_SUBMISSION_LIMIT" in body:
                print(
                    "Concurrency limit reached even after cancellation. "
                    "Waiting for cancellations to complete..."
                )
                time.sleep(30)
                _create_and_submit(client, app_id, version_id)
                return
            else:
                print(f"409 response: {body}")
                print("Already submitted for review, continuing...")
                return
        print(
            f"reviewSubmissions API failed ({e.code}), "
            f"trying appStoreVersionSubmissions...",
            file=sys.stderr,
        )

    # Fallback: legacy appStoreVersionSubmissions API
    try:
        client.post(
            "/appStoreVersionSubmissions",
            {
                "data": {
                    "type": "appStoreVersionSubmissions",
                    "relationships": {
                        "appStoreVersion": {
                            "data": {
                                "type": "appStoreVersions",
                                "id": version_id,
                            }
                        }
                    },
                }
            },
        )
        print("Submitted for notarization (legacy API)!")
    except HTTPError as e:
        if e.code == 409:
            print("Already submitted, continuing...")
        else:
            print(
                f"ERROR: Failed to submit for notarization: {e.code}",
                file=sys.stderr,
            )
            raise


def wait_for_adp(client: ASCClient, version_id: str) -> str:
    """Wait for the alternative distribution package to be generated."""
    elapsed = 0
    while elapsed < MAX_WAIT_NOTARIZATION:
        try:
            # Use the version-level endpoint (build-level returns 404)
            data = client.get(
                f"/appStoreVersions/{version_id}/alternativeDistributionPackage"
            )
            adp = data.get("data")
            if adp:
                adp_id = adp["id"]
                print(f"Alternative distribution package found: {adp_id}")
                return adp_id
        except HTTPError as e:
            if e.code == 404:
                pass  # ADP not yet generated
            else:
                raise
        if elapsed == 0:
            print("Waiting for notarization to complete and ADP to be generated...")
        print(f"  ADP not ready yet (waited {elapsed}s)")
        time.sleep(POLL_INTERVAL)
        elapsed += POLL_INTERVAL
    print(
        f"ERROR: ADP not generated after {MAX_WAIT_NOTARIZATION}s", file=sys.stderr
    )
    sys.exit(1)


def get_adp_versions(client: ASCClient, adp_id: str) -> list:
    data = client.get(f"/alternativeDistributionPackages/{adp_id}/versions")
    versions = data.get("data", [])
    if not versions:
        print("ERROR: No ADP versions found", file=sys.stderr)
        sys.exit(1)
    return versions


def get_adp_variants(client: ASCClient, version_id: str) -> list:
    data = client.get(
        f"/alternativeDistributionPackageVersions/{version_id}/variants"
    )
    return data.get("data", [])


def download_adp(client: ASCClient, adp_id: str, output_dir: Path) -> Path:
    """Download the ADP files to output_dir."""
    output_dir.mkdir(parents=True, exist_ok=True)

    versions = get_adp_versions(client, adp_id)
    print(f"ADP has {len(versions)} version(s)")

    for version in versions:
        version_id = version["id"]
        version_attrs = version.get("attributes", {})
        print(f"  Version: {version_id} state={version_attrs.get('state')}")

        variants = get_adp_variants(client, version_id)
        if variants:
            print(f"  Found {len(variants)} variant(s)")
            for variant in variants:
                attrs = variant.get("attributes", {})
                url = attrs.get("url")
                if url:
                    dest = output_dir / f"variant-{variant['id']}"
                    dest.mkdir(parents=True, exist_ok=True)
                    raw_path = dest / "package.tgz"
                    print(f"  Downloading variant to {raw_path}...")
                    # Download URL is pre-signed, no auth needed
                    req = Request(url)
                    with urlopen(req) as resp:
                        content = resp.read()
                        with open(raw_path, "wb") as f:
                            f.write(content)
                        print(f"  Downloaded {len(content)} bytes")

    # Save ADP metadata
    meta_path = output_dir / "adp-info.json"
    with open(meta_path, "w") as f:
        json.dump({"adpId": adp_id, "versions": versions}, f, indent=2)
    print(f"ADP metadata saved to {meta_path}")

    return output_dir


def main():
    parser = argparse.ArgumentParser(
        description="Submit for notarization and fetch ADP from App Store Connect"
    )
    parser.add_argument("--key-path", required=True, help="Path to ASC API .p8 key")
    parser.add_argument("--key-id", required=True, help="ASC API Key ID")
    parser.add_argument("--issuer-id", required=True, help="ASC API Issuer ID")
    parser.add_argument("--bundle-id", required=True, help="App bundle ID")
    parser.add_argument(
        "--version", required=True, help="Marketing version string (e.g. 1.0.1)"
    )
    parser.add_argument(
        "--output-dir",
        default="./adp",
        help="Directory to download ADP into (default: ./adp)",
    )
    parser.add_argument(
        "--min-build-number",
        type=int,
        default=0,
        help="Wait for a build with version >= this number (0 = use latest)",
    )
    args = parser.parse_args()

    client = ASCClient(args.key_path, args.key_id, args.issuer_id)

    # 1. Find app
    app_id = find_app(client, args.bundle_id)

    # 2. Find latest build (wait for it if just uploaded)
    build = find_latest_build(client, app_id, args.min_build_number)
    build_id = build["id"]

    # 3. Wait for build processing
    state = build.get("attributes", {}).get("processingState")
    if state != "VALID":
        wait_for_processing(client, build_id)

    # 4. Create or find version, set review type to NOTARIZATION
    version_id = find_or_create_version(client, app_id, args.version, build_id)
    set_review_type_notarization(client, version_id)
    attach_build_to_version(client, version_id, build_id)

    # 5. Submit for notarization
    submit_for_notarization(client, app_id, version_id)

    # 6. Wait for ADP
    adp_id = wait_for_adp(client, version_id)

    # 7. Download ADP
    output_dir = Path(args.output_dir)
    download_adp(client, adp_id, output_dir)

    # Output for CI
    github_output = os.environ.get("GITHUB_OUTPUT")
    if github_output:
        with open(github_output, "a") as f:
            f.write(f"adp-id={adp_id}\n")
            f.write(f"adp-dir={output_dir}\n")

    print("\nDone! ADP downloaded successfully.")


if __name__ == "__main__":
    main()
