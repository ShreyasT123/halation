

from pathlib import Path
import requests
import zipfile
import io
import onnx

# Qualcomm's official ONNX float model archive
URL = (
    "https://qaihub-public-assets.s3.us-west-2.amazonaws.com/"
    "qai-hub-models/models/mobilesam/releases/v0.61.0/"
    "mobilesam-onnx-float.zip"
)

OUTPUT_DIR = Path("models/mobilesam")
OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

print("Downloading Qualcomm MobileSAM ONNX...")

response = requests.get(URL, stream=True, timeout=300)
response.raise_for_status()

# Extract archive
with zipfile.ZipFile(io.BytesIO(response.content)) as archive:
    archive.extractall(OUTPUT_DIR)

print(f"Extracted to: {OUTPUT_DIR.resolve()}")

# Find and validate ONNX files
onnx_files = list(OUTPUT_DIR.rglob("*.onnx"))

if not onnx_files:
    raise RuntimeError("No ONNX files found in archive")

for model_path in onnx_files:
    print(f"\nValidating: {model_path}")

    model = onnx.load(str(model_path))
    onnx.checker.check_model(model)

    print("Model is valid")
    print("Inputs:")

    for inp in model.graph.input:
        print(" ", inp.name)

    print("Outputs:")

    for out in model.graph.output:
        print(" ", out.name)

print("\nDownload and validation complete.")
