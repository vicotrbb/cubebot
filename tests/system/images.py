"""Build and remove owned sandbox images in a disposable Docker builder."""

import json
import subprocess


def run(*args, input=None):
    result = subprocess.run(
        args, input=input, text=True, capture_output=True, check=False
    )
    if result.returncode:
        raise RuntimeError(result.stderr.strip())
    return result.stdout.strip()


def docker(*args, input=None):
    return run("docker", *args, input=input)


def build(builder, prefix):
    print("Building isolated sandbox image", flush=True)
    image = f"{prefix}-sandbox"
    docker(
        "buildx",
        "build",
        "--builder",
        builder,
        "--load",
        "--label",
        f"org.cubebot.fixture={prefix}",
        "--tag",
        image,
        "--file",
        "apps/sandbox/Dockerfile",
        ".",
    )
    return {"sandbox": image}


def image_metadata(images):
    metadata = {
        role: json.loads(docker("image", "inspect", image, "--format", "{{json .}}"))
        for role, image in images.items()
    }
    assert metadata["sandbox"]["Config"]["User"] == "10001:10001"
    return {
        role: {
            "id": data["Id"],
            "architecture": data["Architecture"],
            "os": data["Os"],
            "user": data["Config"]["User"],
            "entrypoint": data["Config"]["Entrypoint"],
        }
        for role, data in metadata.items()
    }


def missing_resource(message):
    text = message.lower()
    return any(
        marker in text
        for marker in (
            "no such container:",
            "no such volume",
            "no such image:",
            "no builder ",
        )
    )


def remove_owned(command, resource):
    try:
        result = subprocess.run(command, text=True, capture_output=True, check=False)
    except OSError:
        return f"{resource}: cleanup command unavailable"
    if result.returncode and not missing_resource(result.stderr):
        return f"{resource}: cleanup exited {result.returncode}"
    return None


def cleanup(prefix, builder):
    commands = [
        (["docker", "rm", "--force", prefix], prefix),
        (["docker", "volume", "rm", prefix], f"volume:{prefix}"),
        (["docker", "buildx", "rm", "--force", builder], builder),
        (["docker", "image", "rm", f"{prefix}-sandbox"], f"{prefix}-sandbox"),
    ]
    failures = []
    for command, resource in commands:
        failure = remove_owned(command, resource)
        if failure:
            failures.append(failure)
    return failures
