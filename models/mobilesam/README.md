---
library_name: pytorch
license: apache-2.0
tags:
- foundation
- android
pipeline_tag: image-segmentation

---

![](https://qaihub-public-assets.s3.us-west-2.amazonaws.com/qai-hub-models/models/mobilesam/web-assets/model_demo.png)

# MobileSam: Optimized for Qualcomm Devices

Transformer based encoder-decoder where prompts specify what to segment in an image thereby allowing segmentation without the need for additional training. The image encoder generates embeddings and the lightweight decoder operates on the embeddings for point and mask based image segmentation.

This is based on the implementation of MobileSam found [here](https://github.com/facebookresearch/segment-anything).
This repository contains pre-exported model files optimized for Qualcomm® devices. You can use the [Qualcomm® AI Hub Models](https://github.com/qualcomm/ai-hub-models/blob/v0.65.0/src/qai_hub_models/models/mobilesam) library to export with custom configurations. More details on model performance across various devices, can be found [here](#performance-summary).

Qualcomm AI Hub Models uses [Qualcomm AI Hub Workbench](https://workbench.aihub.qualcomm.com) to compile, profile, and evaluate this model. [Sign up](https://myaccount.qualcomm.com/signup) to run these models on a hosted Qualcomm® device.

## Getting Started
There are two ways to deploy this model on your device:

### Option 1: Download Pre-Exported Models

Below are pre-exported model assets ready for deployment.

| Runtime | Precision | Chipset | SDK Versions | Download |
|---|---|---|---|---|
| ONNX | float | Universal | QAIRT 2.50, ONNX Runtime 1.30.0 | [Download](https://qaihub-public-assets.s3.us-west-2.amazonaws.com/qai-hub-models/models/mobilesam/releases/v0.65.0/mobilesam-onnx-float.zip)
| QNN_DLC | float | Universal | QAIRT 2.50 | [Download](https://qaihub-public-assets.s3.us-west-2.amazonaws.com/qai-hub-models/models/mobilesam/releases/v0.65.0/mobilesam-qnn_dlc-float.zip)
| TFLITE | float | Universal | QAIRT 2.50 | [Download](https://qaihub-public-assets.s3.us-west-2.amazonaws.com/qai-hub-models/models/mobilesam/releases/v0.65.0/mobilesam-tflite-float.zip)

For more device-specific assets and performance metrics, visit **[MobileSam on Qualcomm® AI Hub](https://aihub.qualcomm.com/models/mobilesam)**.


### Option 2: Export with Custom Configurations

Use the [Qualcomm® AI Hub Models](https://github.com/qualcomm/ai-hub-models/blob/v0.65.0/src/qai_hub_models/models/mobilesam) Python library to compile and export the model with your own:
- Custom weights (e.g., fine-tuned checkpoints)
- Custom input shapes
- Target device and runtime configurations

This option is ideal if you need to customize the model beyond the default configuration provided here.

See our repository for [MobileSam on GitHub](https://github.com/qualcomm/ai-hub-models/blob/v0.65.0/src/qai_hub_models/models/mobilesam) for usage instructions.

## Model Details

**Model Type:** Model_use_case.semantic_segmentation

**Model Stats:**
- Input resolution: 720p (720x1280)
- Model checkpoint: vit_t
- Model size (sam_decoder) (float): 23.7 MB
- Model size (sam_encoder) (float): 26.6 MB
- Number of parameters (sam_decoder): 6.16M
- Number of parameters (sam_encoder): 6.95M

## Performance Summary
| Model | Runtime | Precision | Chipset | Inference Time (ms) | Peak Memory Range (MB) | Primary Compute Unit
|---|---|---|---|---|---|---
| decoder | ONNX | float | Snapdragon® 8 Elite Gen 5 For Galaxy Mobile | 2.691 ms | 2 - 216 MB | NPU
| decoder | ONNX | float | Snapdragon® 8 Elite For Galaxy Mobile | 3.1 ms | 0 - 210 MB | NPU
| decoder | ONNX | float | Snapdragon® 8 Gen 3 Mobile | 4.149 ms | 0 - 243 MB | NPU
| decoder | ONNX | float | Qualcomm® Dragonwing™ IQ-8275 | 6.133 ms | 4 - 8 MB | NPU
| decoder | ONNX | float | Qualcomm® Dragonwing™ IQ-9075 | 6.784 ms | 4 - 7 MB | NPU
| decoder | ONNX | float | Qualcomm® Dragonwing™ Q-8750 | 3.1 ms | 0 - 210 MB | NPU
| decoder | QNN_DLC | float | Snapdragon® 8 Elite Gen 5 For Galaxy Mobile | 2.384 ms | 4 - 193 MB | NPU
| decoder | QNN_DLC | float | Snapdragon® 8 Elite For Galaxy Mobile | 2.8 ms | 0 - 188 MB | NPU
| decoder | QNN_DLC | float | Snapdragon® X2 Elite | 2.861 ms | 4 - 4 MB | NPU
| decoder | QNN_DLC | float | Snapdragon® X Elite | 5.821 ms | 4 - 4 MB | NPU
| decoder | QNN_DLC | float | Snapdragon® 8 Gen 3 Mobile | 3.702 ms | 4 - 221 MB | NPU
| decoder | QNN_DLC | float | Snapdragon® 8 Gen 1 Mobile | 7.592 ms | 0 - 212 MB | NPU
| decoder | QNN_DLC | float | Qualcomm® Dragonwing™ IQ-8275 | 5.665 ms | 4 - 11 MB | NPU
| decoder | QNN_DLC | float | Qualcomm® Dragonwing™ QCS8550 (Proxy) | 5.378 ms | 4 - 55 MB | NPU
| decoder | QNN_DLC | float | Qualcomm® SA8650P | 6.389 ms | 0 - 196 MB | NPU
| decoder | QNN_DLC | float | Qualcomm® SA8255P | 6.389 ms | 0 - 196 MB | NPU
| decoder | QNN_DLC | float | Qualcomm® QCS8450 | 7.592 ms | 0 - 212 MB | NPU
| decoder | QNN_DLC | float | Qualcomm® Dragonwing™ IQ-9075 | 6.24 ms | 4 - 10 MB | NPU
| decoder | QNN_DLC | float | Qualcomm® Dragonwing™ IQ-X7181 | 5.821 ms | 4 - 4 MB | NPU
| decoder | QNN_DLC | float | Qualcomm® Dragonwing™ Q-8750 | 2.8 ms | 0 - 188 MB | NPU
| decoder | QNN_DLC | float | Qualcomm® SA8295P | 7.705 ms | 0 - 188 MB | NPU
| decoder | TFLITE | float | Snapdragon® 8 Elite Gen 5 For Galaxy Mobile | 2.238 ms | 0 - 182 MB | NPU
| decoder | TFLITE | float | Snapdragon® 8 Elite For Galaxy Mobile | 2.598 ms | 0 - 180 MB | NPU
| decoder | TFLITE | float | Snapdragon® 8 Gen 3 Mobile | 3.401 ms | 0 - 211 MB | NPU
| decoder | TFLITE | float | Snapdragon® 8 Gen 1 Mobile | 7.238 ms | 0 - 206 MB | NPU
| decoder | TFLITE | float | Qualcomm® Dragonwing™ IQ-8275 | 5.256 ms | 0 - 20 MB | NPU
| decoder | TFLITE | float | Qualcomm® Dragonwing™ QCS8550 (Proxy) | 4.844 ms | 0 - 18 MB | NPU
| decoder | TFLITE | float | Qualcomm® SA8650P | 5.89 ms | 0 - 188 MB | NPU
| decoder | TFLITE | float | Qualcomm® SA8255P | 5.89 ms | 0 - 188 MB | NPU
| decoder | TFLITE | float | Qualcomm® QCS8450 | 7.238 ms | 0 - 206 MB | NPU
| decoder | TFLITE | float | Qualcomm® Dragonwing™ IQ-9075 | 5.766 ms | 0 - 19 MB | NPU
| decoder | TFLITE | float | Qualcomm® Dragonwing™ Q-8750 | 2.598 ms | 0 - 180 MB | NPU
| decoder | TFLITE | float | Qualcomm® SA8295P | 7.192 ms | 0 - 210 MB | NPU
| encoder | ONNX | float | Snapdragon® 8 Elite Gen 5 For Galaxy Mobile | 345.95 ms | 45 - 949 MB | NPU
| encoder | ONNX | float | Snapdragon® 8 Elite For Galaxy Mobile | 383.467 ms | 91 - 761 MB | NPU
| encoder | ONNX | float | Snapdragon® 8 Gen 3 Mobile | 172.741 ms | 102 - 1345 MB | NPU
| encoder | ONNX | float | Qualcomm® Dragonwing™ IQ-8275 | 240.237 ms | 54 - 58 MB | NPU
| encoder | ONNX | float | Qualcomm® Dragonwing™ IQ-9075 | 217.33 ms | 58 - 62 MB | NPU
| encoder | ONNX | float | Qualcomm® Dragonwing™ Q-8750 | 383.467 ms | 91 - 761 MB | NPU
| encoder | QNN_DLC | float | Snapdragon® 8 Elite Gen 5 For Galaxy Mobile | 639.527 ms | 12 - 1438 MB | NPU
| encoder | QNN_DLC | float | Snapdragon® 8 Elite For Galaxy Mobile | 687.388 ms | 0 - 1296 MB | NPU
| encoder | QNN_DLC | float | Snapdragon® X2 Elite | 398.212 ms | 12 - 12 MB | NPU
| encoder | QNN_DLC | float | Snapdragon® X Elite | 831.111 ms | 12 - 12 MB | NPU
| encoder | QNN_DLC | float | Snapdragon® 8 Gen 3 Mobile | 79.943 ms | 12 - 1647 MB | NPU
| encoder | QNN_DLC | float | Snapdragon® 8 Gen 1 Mobile | 203.305 ms | 0 - 1332 MB | NPU
| encoder | QNN_DLC | float | Qualcomm® Dragonwing™ IQ-8275 | 987.704 ms | 12 - 31 MB | NPU
| encoder | QNN_DLC | float | Qualcomm® Dragonwing™ QCS8550 (Proxy) | 966.327 ms | 12 - 836 MB | NPU
| encoder | QNN_DLC | float | Qualcomm® SA8650P | 100.287 ms | 1 - 1549 MB | NPU
| encoder | QNN_DLC | float | Qualcomm® SA8255P | 100.287 ms | 1 - 1549 MB | NPU
| encoder | QNN_DLC | float | Qualcomm® QCS8450 | 203.305 ms | 0 - 1332 MB | NPU
| encoder | QNN_DLC | float | Qualcomm® Dragonwing™ IQ-9075 | 915.472 ms | 12 - 30 MB | NPU
| encoder | QNN_DLC | float | Qualcomm® Dragonwing™ IQ-X7181 | 831.111 ms | 12 - 12 MB | NPU
| encoder | QNN_DLC | float | Qualcomm® Dragonwing™ Q-8750 | 687.388 ms | 0 - 1296 MB | NPU
| encoder | QNN_DLC | float | Qualcomm® SA8295P | 247.754 ms | 1 - 1100 MB | NPU
| encoder | TFLITE | float | Snapdragon® 8 Elite Gen 5 For Galaxy Mobile | 400.866 ms | 4 - 1315 MB | NPU
| encoder | TFLITE | float | Snapdragon® 8 Elite For Galaxy Mobile | 445.047 ms | 4 - 1203 MB | NPU
| encoder | TFLITE | float | Snapdragon® 8 Gen 3 Mobile | 490.256 ms | 74 - 1562 MB | NPU
| encoder | TFLITE | float | Snapdragon® 8 Gen 1 Mobile | 201.436 ms | 4 - 1341 MB | NPU
| encoder | TFLITE | float | Qualcomm® Dragonwing™ IQ-8275 | 637.235 ms | 0 - 50 MB | NPU
| encoder | TFLITE | float | Qualcomm® Dragonwing™ QCS8550 (Proxy) | 627.881 ms | 0 - 4 MB | NPU
| encoder | TFLITE | float | Qualcomm® SA8650P | 100.639 ms | 0 - 1665 MB | NPU
| encoder | TFLITE | float | Qualcomm® SA8255P | 100.639 ms | 0 - 1665 MB | NPU
| encoder | TFLITE | float | Qualcomm® QCS8450 | 201.436 ms | 4 - 1341 MB | NPU
| encoder | TFLITE | float | Qualcomm® Dragonwing™ IQ-9075 | 597.633 ms | 0 - 49 MB | NPU
| encoder | TFLITE | float | Qualcomm® Dragonwing™ Q-8750 | 445.047 ms | 4 - 1203 MB | NPU
| encoder | TFLITE | float | Qualcomm® SA8295P | 247.541 ms | 0 - 1094 MB | NPU

## License
* The license for the original implementation of MobileSam can be found
  [here](https://github.com/facebookresearch/segment-anything/blob/main/LICENSE).

## References
* [Segment Anything](https://arxiv.org/abs/2306.14289)
* [Source Model Implementation](https://github.com/facebookresearch/segment-anything)

## Community
* Join [our AI Hub Slack community](https://aihub.qualcomm.com/community/slack) to collaborate, post questions and learn more about on-device AI.
* For questions or feedback please [reach out to us](mailto:ai-hub-support@qti.qualcomm.com).
