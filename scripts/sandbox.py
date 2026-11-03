"""Signing configuration for the isolated App Sandbox development slice."""
import re


def configuration(app_id, team_id):
    if not re.fullmatch(r"[A-Z0-9]{10}", team_id):
        raise ValueError("Sandbox team ID must contain exactly ten uppercase letters or digits.")
    group = f"{team_id}.{app_id}.shared"
    common = {
        "com.apple.security.app-sandbox": True,
        "com.apple.security.application-groups": [group],
        "com.apple.security.files.bookmarks.app-scope": True,
        "com.apple.security.files.user-selected.read-write": True,
        "com.apple.security.network.client": True,
    }
    application = dict(common)
    # Sparkle's installer/status endpoints are the only global lookup exceptions.
    application["com.apple.security.temporary-exception.mach-lookup.global-name"] = [
        app_id + "-spks", app_id + "-spki"]
    engine = dict(common, **{"com.apple.security.network.server": True})
    return {"group": group, "application": application, "engine": engine}
