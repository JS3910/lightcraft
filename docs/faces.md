# Faces

LightCraft reads the face names other apps (Lightroom, digiKam…) write into your photos, shows them in the loupe
(boxes you can switch off, resize and remove; the loupe toolbar, beside the face-boxes button, says who is in the photo: the names left to right, each once,
then how many faces are not named, and a click on that text shows or hides the boxes) and groups photos by person in the People view. This page is about the
*models* that find and recognise faces by themselves.

## Nothing is bundled; every model is an opt-in download

No model weights are part of LightCraft. Each one is a file you choose to get, from **Settings ▸ Faces**:

- **Face detection:** [YuNet](https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet) (232 KB, MIT).
  It runs in LightCraft's own code, with no extra runtime. It was trained on the WIDER FACE dataset; an MIT licence on
  weights does not settle that dataset's terms, so this is flagged for the maintainer. It finds faces; it does not
  identify anyone. **Photo ▸ Detect Faces** offers the download the first time you use it.
- **Face recognition models** are large (AuraFace is 261 MB), and their licences and training data deserve a decision by
  the person installing them. Everything works without one: names from XMP, the People view, the face boxes.
- **Recognition runs in LightCraft's own pure-Rust CPU interpreter**, shared with the detector. Dense
  convolutions use bounded im2col and single-thread float32 GEMM (ndarray 0.17.2 / matrixmultiply 0.3.11, MIT or Apache-2.0, default/BLAS/threading features disabled);
  depthwise convolutions use row-wise Rust loops. There is no nested thread pool. No C or assembly is compiled by
  this dependency path. Desktop and CLI builds always include recognition (there is no separate cargo feature for it). The browser retains its existing model-install/background-scan restrictions.

### Supported graphs and limits

The pinned SFace 2021dec and AuraFace v1 files both use ONNX opset 11. Their combined operators are **Conv** (including
SFace's depthwise groups), **BatchNormalization**, **PRelu**, **Add**, **Sub**, **Mul**, **Dropout** (inference identity),
**Flatten**, and **Gemm**. The shared engine also supports the detector's Relu, Sigmoid, MaxPool, nearest whole-number
Resize, Transpose and constant-shape Reshape, plus GlobalAveragePool and Identity. This is a limited inference engine:
other operators (including MatMul and Concat), custom domains, training graphs, unsupported attribute combinations,
external/sparse weights and non-float32 inputs/outputs fail to load with an explicit error.

Shapes and attributes are checked before installation's self-test, including all intermediates and the promised output
size. Files are limited to 512 MiB, combined weights to 64 Mi elements (256 MiB of float32), rank to eight and live tensors plus
retained convolution scratch to 512 MiB. All supplied weights and produced activations must be finite. Embeddings retain the
manifest's RGB/BGR and mean/std preprocessing, and are normalized using a bounded float64 norm to avoid float32 overflow.

The agreement figures below come from a one-off comparison against an independent ONNX reference runtime, run outside
this repository; that tooling is not part of it, so those figures cannot be reproduced from here. What the repository
does check is listed under "Agreement and regression coverage".

## Downloading a model

**Settings ▸ Faces ▸ Download** (or Photo ▸ Detect Faces, which offers it for the detector):

1. A dialog shows the model's licence, whether commercial use is allowed, what it was trained on (or that this is not
   known) and which site the file comes from. **Download stays disabled until you tick "I have read these terms and
   accept them for my own use".** Nothing is fetched before that.
2. LightCraft downloads the file from the model's own repository (GitHub for YuNet and SFace, Hugging Face for
   AuraFace), at an address pinned to a commit, over https, in pure Rust (the `lightcraft-fetch` crate: no `curl`, no
   OpenSSL). An interrupted download resumes where it stopped.
3. The file must match the model's recorded size and SHA-256, or it is thrown away. A file that matches is installed
   by itself. Nothing else is sent anywhere, and no token or account is involved.

**Open page** opens the model's own page in your browser if you would rather get the file yourself.

## Adding a model file yourself

Adding a model is how you say you want it: once it is installed it is **chosen and recognition is switched on**, with no
further step. The one question is its licence, and that comes first.

1. **Settings ▸ Faces ▸ Add a model file…**, or drop a `.onnx` file on the window.
2. LightCraft looks at the file (it never runs it at this point): it recognises models it knows by their SHA-256, and
   for any other file it reads the input and output shapes and describes what it assumed (112 × 112 aligned faces,
   RGB, `(x − 127.5) / 127.5`, one vector per face: the ArcFace convention that InsightFace models also use).
3. The same dialog shows the licence, whether commercial use is allowed and what the model was trained on, and
   **Install stays disabled until you tick the acceptance box**.
4. The model is copied into LightCraft's models folder and checked against the original by hash.

Non-commercial models (InsightFace, for example) can be added this way for your own use; LightCraft never bundles,
hosts, downloads or links them from a picker, and the dialog says so.

The newest model is the one in use. An earlier one stays installed, **Use** switches back, and each model keeps its own
cache of embeddings (`face-embeddings-<model id>.bin` in the library folder), so going back does not start over. Removing
the model in use hands over to another installed one. Results of the model that was running when you switched are
discarded, never mixed into the new one's index.

### Bring your own models: `catalog.json`

LightCraft's built-in list only holds models whose terms let the project point at them. For anything else (a stronger
recogniser whose weights are for research use, a model you trained, a mirror you trust) put a `catalog.json` in the models
folder. Each entry is a model manifest plus the address to fetch it from, and it then gets the same **Download** button as
a built-in model, with its licence notice shown before anything is fetched. LightCraft ships and links to none of them: what
the file says, and whether you may use the weights, is yours to check (the dialog shows the notice you wrote, in a warning
colour unless you marked the model `"commercial": "yes"`).

```json
{ "models": [ {
    "id": "my-recogniser", "name": "My recogniser (R50)", "version": "1", "role": "embedder",
    "url": "https://example.org/weights/recogniser.onnx",
    "sha256": "(64 lowercase hex digits)", "sizeBytes": 166000000,
    "licence": { "name": "Research use only", "commercial": "no", "notice": "Not for commercial use." },
    "provenance": "Trained on ...",
    "output": { "kind": "embedding", "dim": 512 },
    "thresholds": { "matchCosine": 0.4 },
    "speed": 1.7
} ] }
```

`speed` is optional: how many times faster the model is than a ResNet-100 (1.7 for a ResNet-50, 7 for a MobileFaceNet; see the
next section). Settings ▸ Faces shows it as "1.7× faster than a ResNet-100 model", also before the model is downloaded; leave it
out if you have not measured it and nothing is shown. `input` may be left out (112 × 112 RGB, `(x − 127.5) / 127.5`, the ArcFace convention that InsightFace-style models use); say
otherwise with `"input": {"width": 112, "height": 112, "colour": "bgr", "mean": [0,0,0], "std": [1,1,1]}`. `sha256` and
`sizeBytes` are required (a download is checked against them), the address must be `https`, and the model must give one
vector per face. A bad entry is reported in Settings ▸ Faces and skipped; the rest still load. The same file by hand
(Add a model file…) is recognised by its hash and gets the terms you wrote. Models you cannot or do not want to list can
still be added by file; non-commercial models are never offered by the built-in list.

The models folder is `<config>/models` (`%APPDATA%\LightCraft\models` on Windows, `~/Library/Application Support/LightCraft/models`
on macOS, `~/.config/lightcraft/models` on Linux), or `$LIGHTCRAFT_FACE_MODELS`. The desktop app, the CLI and the MCP
server share it.

## What is known about the models LightCraft recognises

| Model | Licence of the weights | Trained on | Notes |
| --- | --- | --- | --- |
| YuNet 2023mar (detector) | MIT | WIDER FACE | 232 KB |
| AuraFace v1 | Apache-2.0 | "a commercial dataset", undisclosed | 261 MB; the best measured on sculpted busts |
| SFace 2021dec | labelled Apache-2.0 | undocumented (the upstream repository mentions CASIA-WebFace, VGGFace2, MS1MV2) | 39 MB; two questions about commercial use are unanswered upstream |

"Commercial use allowed" in the dialog is the weights' licence; it says nothing about the training data.

## How fast are the models

The current pure-Rust runner was measured in standard release mode (no `target-cpu=native`) on Windows,
Ryzen 9 7945HX, over **40 aligned photographic crops** from `corpus/public-faces` (public-domain sources in
`SOURCES.tsv`). Each call runs on one thread. Preprocessing, inference and unit normalization are included;
model loading, photo decoding, detection and alignment are excluded.

| Model | Pure Rust / face (median) | Speed (R100 = 1×) |
| --- | ---: | ---: |
| SFace 2021dec (39 MB) | **33.82–36.29 ms** | **6.9×** |
| AuraFace v1 (261 MB) | **233.18–252.14 ms** | **1×** |

The ranges are medians from two runs (the rerun reproduced the same embedding errors).
Five faces cost about **169–181 ms with SFace**, or **1.17–1.26 s with AuraFace**, just for recognition; ten faces cost
338–363 ms / 2.33–2.52 s. Decoding, detecting and aligning the photo add to this. The background scan parallelizes photos
using its existing bounded worker pools; the recognizer never creates an inner pool. These serial estimates do not imply
whole-library throughput. Convolution dominates the profiled model time. Dense kernels lower at most 256 positions
with contiguous plane copies and explicit border masks; depthwise kernels remain direct. CPU-dispatched Rust
AVX-512 intrinsics improve supported x86 CPUs; other CPUs and wasm use the dependency's Rust fallback.
AuraFace remains a costly CPU choice (its load time is 120–172 ms).

SFace's builtin speed metadata now uses the measured 6.9× ratio. User `catalog.json` ratios remain user-provided.
Older published tables for MobileFaceNet/R50/R100 came from a different runtime and are not measurements of this runner; architecture
names alone do not guarantee the same speed, and those additional model files were not verified in this task.

### On the GPU

On a machine with a usable GPU, the whole detector and recogniser run on it through wgpu (`crates/gpu/src/faces.rs`).
The interpreter lowers the graph to a flat program for the input's shape (`crates/faces/src/net/program.rs`): each
convolution (`wgsl/faces_conv.wgsl`: an implicit matrix product that gathers the input straight into workgroup memory,
cuts K into slices when a map is small, and a direct kernel for depthwise layers) and every other operator (activations,
batch norm, PRelu, broadcasting add/sub/mul, max pooling, nearest upscaling, transposes, global pooling and the fully
connected layer, `wgsl/faces_ops.wgsl`) is a kernel, every value stays on the device between layers in reused
buffers, and a run is one upload, one submission and one read-back. Shape-only operators make no step. A model that
needs something the program cannot express runs its convolutions one by one on the GPU instead. Several faces of one
photo go together (`Embedder::embed_batch`, up to 16 per submission).

The CPU interpreter remains the reference. After a model loads, a background check (a row in the activity stack, "Checking
the GPU for faces") runs it once on the CPU and twice through the GPU on a fixed test input: the GPU is used only when
every output is within 1e-3 of the CPU's (relative to the output's largest value) **and** it was faster; until then,
and whenever the GPU is switched off (`LIGHTCRAFT_GPU=0`, `LIGHTCRAFT_GPU_BACKEND=off`, the GPU preference, or **Settings ▸
Faces ▸ Run face models on the CPU only**), missing, wrong or slower, the model runs on the CPU. The first GPU error,
or a result of the wrong size or with values that are not finite, drops it for that model. Settings ▸ Faces shows
which device each model runs on (`faces.gpu`). The weights stay on a device of the scan's own (separate from the
editor's) while the model is loaded, at most 768 MB per model.

The background scan's pace also limits the GPU: paused or light, one model run at a time and the GPU busy at most a
quarter of the time (the editor renders on the same GPU); normal, two at once and three quarters; full, as fast as it
goes.

Measured on Linux with a Radeon (Vulkan) and a 32-thread CPU, one call at a time, machine under very heavy load from
other jobs (load average 100–250, so the absolute numbers are noisy):

| Model | CPU | GPU, one layer at a time | GPU, whole network | Batch of 16, per face |
| --- | ---: | ---: | ---: | ---: |
| YuNet (640², one photo) | 58–60 ms | 85–89 ms | 21 ms | – |
| SFace 2021dec | 40–42 ms | 47–51 ms | 9.6 ms | 8.1 ms |
| AuraFace v1 | 295–308 ms | 165–172 ms | 70–73 ms | 71 ms |

`LC_FACE_MODELS=<folder> cargo test --release -p lightcraft-gpu faces::tests -- --include-ignored --nocapture` checks
agreement on the real models and every operator, and prints these timings.

### Agreement and regression coverage

The public-crop embeddings match the reference runtime's outputs:

| Model | Maximum absolute component difference | Mean absolute difference | Minimum cosine | Mean cosine |
| --- | ---: | ---: | ---: | ---: |
| SFace | 2.0266e-6 | 2.0542e-7 | 0.999999999992 | 0.999999999994 |
| AuraFace | 1.2219e-6 | 1.2725e-7 | 0.999999999990 | 0.999999999993 |

Maximum component difference relative to the largest reference component was 8.60e-6 / 7.10e-6, below 1e-3.
Twenty seeded synthetic recognition nets also agree (maximum absolute embedding difference 7.45e-7).
Unit regressions cover normalization, PReLU broadcasting, transposed Gemm/optional bias, odd grouped convolutions,
reuse of padding, retained scratch/constant-copy limits, invalid manifests, overflow, concurrent calls, and malformed/truncated/ambiguous ONNX fields.
These establish inference agreement, not face-identification accuracy or Lightroom parity.

The shared YuNet graph is also compared against the reference runtime on four synthetic pictures. It passes an absolute
1e-6 plus relative 1e-3 tolerance. The reference's sigmoid uses a clamped rational approximation; LightCraft keeps its existing
exponential sigmoid. Tiny confidence outputs differ by at most 4.47e-7 absolute (6.44% relative on the blank picture),
far below the 0.6 detection threshold. Neither pinned recognizer uses Sigmoid. No approximate activation or reduced
precision was introduced to get the speed figures.

`LC_FACE_MODELS=<folder> cargo test --release -p lightcraft-faces runtime::tests::real_models_pass_the_self_test -- --ignored --nocapture`
runs the installation checks against locally supplied, hash-recognized model files. Models/crops stay out of git.

### And how well do they recognise?

Published numbers only, each copied from the model's own page (we have not re-run them). Higher is better. They are
standard face-verification benchmarks: **IJB-C** (hard, mixed-quality photos; true-accept rate at one false accept in
10,000), **CFP-FP** (frontal against profile) and **AgeDB-30** (years apart), both in percent.

| Model | Speed (R100 = 1×) | IJB-C | CFP-FP | AgeDB-30 | Source |
| --- | --- | --- | --- | --- | --- |
| MobileFaceNet (InsightFace buffalo_s / buffalo_sc) | 7× faster | 95.02 | 98.00 | 96.58 | InsightFace model zoo |
| SFace (OpenCV Zoo) | 6.9× faster (current Rust) | not given | not given | not given | OpenCV Zoo says 0.9940 on a set it does not name |
| ResNet-50 (InsightFace buffalo_l) | 1.7× faster | 97.25 | 99.33 | 98.23 | InsightFace model zoo |
| ResNet-100 (AuraFace v1) | 1× | not given | 95.19 | 96.10 | its Hugging Face page |
| TopoFR R50 / R100 / R200 (Glint360K) | 1.7× / 1× / 0.5× | 97.27 / 97.60 / 97.84 | not given | not given | TopoFR repository |

Read with care: the sources report different sets, so the blanks are not zeros and the rows are not all on the same
yardstick. What the numbers do say: a ResNet-50 recogniser is about two points above a MobileFaceNet on IJB-C, a ResNet-100
or 200 adds only a few tenths more, and AuraFace's published CFP-FP and AgeDB-30 are below buffalo_l's ResNet-50 despite being
a ResNet-100 (so it is not the accuracy choice its size suggests). InsightFace's pretrained weights are, in its own words,
"available for non-commercial research purposes only"; TopoFR's page states no licence for its weights.

## Finding faces yourself

**Photo ▸ Detect Faces** (`faces.detect`) runs the YuNet detector on the selected photos and adds what it finds as
unnamed face boxes, in one undo step. A new run replaces earlier detections; boxes that came from XMP, or that you drew or
named, are never touched (and a face that already has one is not boxed a second time), and no sidecar is written. It looks at the photo upright and uncropped with default settings,
so your edits and crops do not matter. `apply: false` only reports. It finds faces of about 10 pixels and up in a
640-pixel version of the photo (so very small faces in a large group photo can be missed; looking at tiles is planned).
The detector's output matches OpenCV's own YuNet on a 45-photo public-domain test set (97 of 99 faces found by both,
mean box overlap 0.97), including marble busts and paintings, with no false boxes on the landscape and architecture
photos in the set.

## Suggesting who is in a photo

With **Recognise faces** switched on in Settings ▸ Faces and a recognition model chosen (**Use**; the switch cannot be
turned on before one is, and turns itself off when the last recogniser is removed), LightCraft works out, in
the background, what each face in your library looks like to the model, and uses the faces you have already named to
suggest names for the ones you have not:

- An unnamed face with a good match gets a dim label such as **Jane Doe?** in the loupe. Click it and the name box opens
  with the guess filled in and selected, so the first letter you type replaces it; Enter confirms it. Any other unnamed face shows **Add name** when you point at it. Clicking a name
  opens the box on it the same way. Clearing the box of a named face and pressing Enter removes the name.
  Pointing at the label of a suggested face (box or label) also shows a check to the left of the name and a circle with a slash to its right, inside the label: the check names the face with the suggestion, the circle says it is not that person (see Rejecting a name); a named face and a face with no suggestion show neither, and the corner × still removes the box in one click.
- **The name box** completes what you type from the people you have already named, in the box itself with the completed
  part selected: Backspace removes it, so a new name that begins like an existing one can still be typed. A short list of
  matching names (at most seven; names that begin with what you typed before names where a later word does) sits below,
  best match first: within each group the people the face looks most like come first, each with its similarity as a
  percentage, and the box completes to the first one (`faces.rank` gives the ranking of every named person for a face;
  it uses only what the background scan has already embedded and never embeds anything). A face the scan has not looked
  at, or recognition being off, gets the matching names alphabetically, without percentages. Up and Down walk the list, a
  click picks one. Enter saves (the listed
  name, else the text), Esc cancels and restores what was there, a click outside saves text you changed and otherwise just
  closes the box. Tab and Shift-Tab save what you typed and move to the next or previous face of the photo; they never
  confirm a suggestion you did not touch. A name that differs from an existing person only in capitalization is stored
  with that person's spelling, unless the face already has that name (so you can still correct a capitalization). A new
  name makes a new person without a question. The box is one component (`crates/ui-egui/src/widgets/name_box.rs`) the
  People view reuses.
- **Nothing is ever named for you.** A suggestion is only a label until you confirm it, and a suggestion appears only when
  the match is strong *and* clearly ahead of the next person: it is better to leave a face unnamed than to name it wrongly.
  Naming a face also makes it one of the faces the others are compared with, so the suggestions improve as you go.
- Faces come from the names other apps wrote into your photos (read from XMP), from Photo ▸ Detect Faces, or both. A face is
  aligned using the detector's five landmarks (eyes, nose, mouth corners) when it finds the same face, and cut out by its box
  otherwise.
- Everything stays on your computer. The embeddings (one short list of numbers per face) are cached in the library folder in
  `face-embeddings-<model id>.bin`, one file per model, so switching models back and forth does not start over; they are
  not part of the catalog and not written to XMP.

### The background scan

Turning recognition on starts a scan of the whole library, once, in the background:

- A photo that already has face boxes (from XMP, or drawn, or named) has its faces embedded.
- A photo with **no** face boxes at all is searched with the detector, and what it is sure of becomes that photo's unnamed
  face boxes (marked "Detected by YuNet", so a manual Detect Faces replaces them), embedded in the same pass. Photos are
  searched once: `face-scanned.bin` in the library folder remembers which, so a photo whose boxes you remove is not boxed
  again. These boxes are LightCraft's own bookkeeping: not an undo step, never written to a sidecar.
- Photos you have named faces in go first, then the rest. Raw files are read through the camera's embedded preview (much
  faster than decoding the raw); everything else is rendered at 2048 pixels.
- **How hard it works follows what you are doing.** The app tells the engine on every call to `faces.pump`, which is made
  about 20 times a second while there is work and a few times a minute otherwise (the window is not redrawn for it at any
  other time). Dragging, typing or scrolling: nothing new is started. The pointer moving, or the window minimized:
  **light**, one photo at a time on two threads. Idle for three seconds, or the window still on screen but with another
  app in front (you are working elsewhere): **normal**, half of the processor's threads. Idle and looking at the progress
  (Settings ▸ Faces, or the People view): **full**, four fifths. Photos already running are never interrupted, and a
  change of pace takes effect at once.
- **Threads and photos.** Most of a photo's cost is its parallel work (decoding, developing the picture), so the pace sets the
  size of a pool of threads for that work: two, half the machine, four fifths. The scan has pools of its own, not the one the
  loupe and exports use (which has no priorities), so a slider drag never queues behind a scan. Earlier measurements (with a different runner) on a 32-thread
  desktop with 4 photos at once: about 6.4 photos a second at full pace (a pool of 25 threads), 6.0 at normal (16) and 1.8
  when light (2 threads).
- **Memory** limits the number of photos at once as much as the processor does: a photo in progress holds about 180 MB
  (peak memory grew by that much per extra worker), so at most half of the memory budget (a quarter of the RAM, at most
  1.5 GiB, unless `LIGHTCRAFT_MEMORY_MB` says otherwise) is given to them: about four photos at once on a default setup,
  whatever the core count. Eight at once was not much faster and needed about twice the memory (a 2.0 GB peak against
  1.1 GB). `LIGHTCRAFT_FACE_THREADS` replaces these limits with a number of your own. A couple more photos wait behind
  the running ones, so a worker that finishes has its next photo at once.
- **Earlier scan speed (a different runner).** A mixed raw and JPEG library of 184 photos (the raws read through their embedded previews, 73 photos searched
  for faces) took about 29 seconds on that machine at full pace: roughly 6 photos a second, so the first scan of 10,000 photos
  is a matter of half an hour; after that only new photos are looked at. Settings ▸ Faces shows how many are left, with a progress bar, and the activity stack shows "Finding faces" meanwhile. After you accept a model's terms
  you are returned to that tab, so the download and then the scan can be watched there; the main window has no status bar.
- Opening another library starts a fresh scan state; nothing learned about one library is used in another.

### The People view

Until recognition is set up, a line at the top says what it takes and has one button: **Set up face recognition** (no model
yet: it opens Settings ▸ Faces, where the download is) or **Turn on** (a model is installed, recognition is off: it switches it
on there and then). The loupe's name box offers the same ("Set up name suggestions…" / "Turn on name suggestions"), and so
does a person's page.

**People** shows a card for each named person (their face, with the number of photos they are in on the picture) and, below
a line, the **Unnamed faces**: every face nobody has named, as cropped pictures. With recognition running, the faces the
named ones recognise come first, the most confident suggestion first (different names mixed, ties by photo and then by
face), each carrying its suggested name on its name strip; then the faces nothing is suggested for, faces that look alike
next to each other (put in order by looking at every pair, for up to the first 1,500 of them); then the faces not
embedded yet, in photo order. Every embedded unnamed face is scored against the named people for this order, on every
core, not only the ones listed (see *The cost of the order* below). Every tile has that strip under its picture, always
shown: the suggested name followed by a question mark (**Jane Doe?**), or a lone **?** when there is no suggestion. Faces are
named in place: click a strip and the shared name box opens on that tile (a suggestion is filled in and selected, so the
first letter you type replaces it); Tab and Shift-Tab move the box to the next and previous tile without confirming
untouched suggestions. Point at a tile with a suggestion and a check and an up arrow appear on the picture: the check
confirms the face as its suggestion (when the tile is selected, every selected face that has a suggestion, each as its own),
and the up arrow confirms every suggestion from the first tile through this one, each face as its own suggested name
(skipping faces without one); each is one undo step, and a tile without a suggestion shows neither. A tile outside the
selection is acted on alone and the selection stays. Click a picture to select it (Shift-click selects a range, **Select all** takes every one listed;
Esc with no box open clears the selection), double-click to open the photo. With faces selected, the box on a selected
tile acts on all of them: type a name and Enter names every selected face, and Enter on an untouched box gives each
selected face its own suggestion and leaves selected faces without one alone. A box on a tile outside the selection
names that face only and leaves the selection as it is. Each is one undo step. Selecting names nothing; there is no
naming bar above the faces.

Click the name under a person's card to rename them in place: the shared name box opens there with the name selected
and offers the people already named (no face to rank them by, so no suggestions). A name nobody has renames every face
of that person at once, as one undo step; a change of capitalization alone counts. The name of another person asks
first: a confirmation that states both names and how many faces each has, with **Combine** and **Cancel**. Combine makes
one person in one undo step (Undo separates them again); Cancel and Esc leave everything as it was. A click on the
picture still opens the person's page.

Right-click a person's card, or use **Rename Person…** on their page, to rename every face of that person in the library
at once (`faces.renamePerson`). The name is matched the way the cards are grouped (trimmed, ignoring capitalization) and
every matching face is stored with the exact new spelling, so a change of capitalization alone updates them all. The
dialog says how many faces that is. If the new name is already someone else, it refuses until **Combine with the existing
person** is ticked; that makes one person, and Undo separates them again. Cancelling changes nothing. A person filter,
the open person page and the way back to it follow the new name. Nothing is written back to the files.

The thumbnail-size slider in the bottom bar sizes the faces here too, with limits of their own. Every face is shown by the
same kind of box: the detector's own box once the scan has looked at it (kept beside its embedding), so a loosely drawn box
from another tool does not make one face look farther away than the next; before the scan has looked at a face it is shown
by its own box.

#### The cost of the order

Putting the Unnamed faces in confidence order scores every embedded unnamed face in scope against every named person
(the mean of the two closest faces of each person, as the name box ranks them), on every core, whether or not the face is
listed: the work is unnamed faces times named faces times 512 numbers. Measured in a release build on a 32-thread
machine with synthetic embeddings (`cargo test --release -p lightcraft-engine --lib face_unnamed_cost -- --ignored
--nocapture`), one `faces.unnamed` call took:

| Unnamed faces | Named faces (people) | Time |
|---|---|---|
| 10,000 | 3,000 (300) | 0.42 s |
| 30,000 | 3,000 (300) | 0.80 s |
| 60,000 | 3,000 (300) | 1.9 s |
| 30,000 | 6,000 (600) | 2.2 s |

The look-alike chain over the faces without a suggestion is capped at 1,500 faces, so it adds a fixed cost. The People
view asks for the list when it opens and when something changes, not every frame.

### A person's page

In **People**, a click on a person opens their page: **only cropped faces**, never whole photos. First the faces named
with their name, then **More**: the unnamed faces that look like them, most alike first. Both use the same tile as the
Unnamed faces: a picture with a name strip under it that is always shown (their name on a confirmed face, their name and a
question mark on a More face). A click on the strip opens the name box: on a More face, type another person's name and press
Enter to give the face to them (it leaves this person's More), or press Enter on the untouched box to confirm it as this
person; on a confirmed face it corrects that one face's name, and emptying the box removes the name. A click on a picture
selects it (Shift-click a range; a double click opens the photo, Escape comes back to the page; Escape with faces selected
clears the selection first), and the box on a selected tile names the whole selection in one undo step. A selection belongs
to one gallery at a time: the Unnamed faces, this person's faces or their More; selecting in another replaces it.
On a More face the hover buttons are the check, which confirms it (one undo step, it moves up into their faces), and `×`, which
says this face is not them (see Rejecting a name below). The `↑` control at the top right of a suggested face confirms that face and every currently shown suggestion
with an equal or higher similarity score. Its tooltip names the person and shows the number of faces before you act.
Hover a suggestion or focus it with the keyboard and press `A` for the same action. The batch is one undo step;
suggestions below that score, previously confirmed faces and faces you rejected stay as they were. Nothing is named
without this explicit action.
**Show photos** puts their photos in the grid; **‹ People** or Escape goes back. "More" only holds faces the scan has
already embedded, so it fills in as the scan goes; it shows faces scoring above four fifths of the model's suggestion bar,
since you look at each one before anything is named.

### Rejecting a name

`×` on a face tile says "this is not that person", and LightCraft remembers it. It is on every tile of the Unnamed faces and of a person's page: on a More face or an unnamed face with a suggestion it rejects the suggested name; on a confirmed face it removes the name and rejects it (the face is then no longer that person's); on a tile with nothing left to reject it removes the face box (see below). The rejected name is kept on the face
itself, in the library only (it is never written into a sidecar or a photo file, and a catalog written before this
existed simply has none). The tile stays where it is with its strip turned to a question mark until you leave the page,
and nothing new is suggested straight away. The face never comes back to that person's More; its next visit to the
Unnamed faces shows its next-best suggestion when one is good enough, and a rejected name is never suggested for it
again. In the name box the names a face rejected are offered last, and typing one anyway names the face and drops the
rejection. Renaming a person, or merging two, moves the rejections to the new name in the same undo step. No face is left rejecting the name it carries: a face that is renamed or merged into a name drops a rejection of that name in the same step, and `faces.reject` leaves a face already called the rejected name alone (it reports it as `skipped`) unless `clearName` is true, as the × on a confirmed face sets it. Edit, Undo takes
a rejection back in one step, however many faces it covered. Re-reading a photo's metadata from its sidecar replaces its
regions with the file's but keeps the rejections of every box that is still there (the same box, to within its overlap), and
synchronizing a folder does the same.

In the loupe the circle with a slash on a suggested label does the same for that one face, in one undo step; the label then shows no suggestion until the photo is shown again, when a next-best one may appear, and the rejected name is never suggested for the face again, in the loupe or in the People view.

### Removing a face box in two steps

Reject works the same on every face tile, in two steps. The first click on a tile that shows a name (a suggestion, or a
confirmed face's own name) rejects it and leaves the tile where it is, its strip turned to a question mark, until you leave
the view; a second click, or the first on a face with no suggestion, removes the face box from the photo. A false detection
is therefore gone in two clicks without leaving the gallery. The removal is catalog only (the sidecar is never rewritten),
and the background scan does not detect the box again: a photo that lost its last face box is remembered as searched.

On a selected tile, reject (and the Delete key, when no name box or other text field has the keyboard) acts on the whole
selection; on an unselected tile, on that face alone. Over a selection it has two levels: while any selected face shows a
name it only rejects those names and removes nothing, and once none does it removes the face boxes of all of them. However
many faces and photos that covers (several boxes of one photo included), it is one undo step, and Edit, Undo brings every
box back with its name and rejections. Face boxes are numbered by their place in the photo's list, so removing one moves
the later boxes of that photo up; the command removes all of a photo's boxes in one edit, and the People view moves its
selection and the tiles it keeps as question marks along with them. Undo and Redo of a removal bring the boxes back at their old places, so after either the view lets go of the selection, the tiles kept as question marks and an open name box rather than point them at another face.


How well it works: on 755 named faces of marble busts from one museum folder (each face hidden in turn and matched against
shots taken more than five seconds apart), SFace named the right person first 95.5% of the time and AuraFace 94.7%; at
the starting thresholds LightCraft suggests (SFace 0.55, AuraFace 0.40) about 97% of the suggestions were right. Busts are
a hard case in some ways (no skin or hair to go by) and an easy one in others (the same sculpture looks the same in every
shot), so check a model on your own photos before trusting it:

`faces.evaluate` tests a model on *your* photos: it hides each named face in turn, asks who it looks like from the others
(ignoring shots taken within a few seconds of it, which would make it too easy) and reports, for each threshold, how many
suggestions it would make and how many were right.

## Face pictures in the People view

Each face on a person card, in Unnamed faces and on a person's page is a square around the face, about 1.7 times the
face box, cut out of the photo with the photo's own look (`Session::face_job`). Where the pixels come from:

- **An unedited raw** (it still looks the way it was imported, the case in which its grid thumbnail is the camera's
  embedded JPEG too) is cut from its **largest embedded preview** (`crates/engine/src/face_tiles.rs`). The preview is
  read the planned way ([raw-preview-reads](raw-preview-reads.md)): the file's header and the JPEG, never the sensor
  data, and the raw is not developed. The preview is used when it holds the square at **at least ¾ of the pixels the
  tile asks for** (up to the 2,560 px the develop path would use); the square is then enlarged by at most 4⁄3, which
  is not visible at tile size (the camera's JPEG is sharpened, so its pixels carry more detail than a binned raw's),
  and is the bound the raw path already accepts when it bins an X-Trans sensor for a preview. A smaller preview, one of
  another shape than the photo (letterboxed, another aspect ratio), one that does not decode, or a file that is offline
  leaves the face to the develop path below.
- **Everything else** (an edited raw, whose edits the camera's JPEG can't take the way the raw does; JPEG, HEIC and
  other files; a raw whose preview is too small) is a develop render of the photo with its crop replaced by the square,
  from the smallest source level that keeps the face sharp, as before.

Measured on two Sony raws on an external 5400 rpm disk (file cache dropped before each face, a centred face box of 4 %
and 10 % of the frame, 176 and 352 px tiles): an α1 ARW (74 MB, 8640 × 5760 Adobe RGB preview) read 70.6 MiB in
0.77–0.92 s per face before and reads 7.8 MiB in 0.20–0.26 s; an A7S III ARW (25 MB, full-size 4240 × 2832 preview)
read 24.3 MiB in 0.38–0.56 s and reads 1.1 MiB in 0.04–0.09 s. The tiles differ from the develop path's by 2–6 levels
of 255 on average (the camera's rendering against LightCraft's camera look, which is fitted to it).

Rendered faces are kept like other thumbnails: in memory, and in the library's thumbnail cache on disk (`thumbs/`,
within the Settings ▸ Performance cache budget, least recently used files go first). The key is the photo's content
hash, its develop settings with the face's square as the crop, the tile size, the renderer version and the camera
profiles, so an edit, another square or a new renderer makes a new picture, and the next launch shows the faces
without reading the originals. Tile sizes are rounded up to a multiple of 16 px, so moving the thumbnail slider
reuses the pictures of its step instead of rendering every face again.

## Commands

All of this is reachable from the control channel, the CLI and MCP: `faces.models.list`, `faces.models.inspect {path}`,
`faces.models.install {path, acknowledged: true, activate?}`, `faces.models.download {id, acknowledged: true}` (then `faces.models.downloads`, which also installs what has arrived, and `faces.models.downloadCancel {id}`), `faces.models.remove {id}`, `faces.models.select {id}`,
`faces.enable {enabled?}`, `faces.detect {ids?, apply?}`, `faces.index {budgetMs?, ids?}`, `faces.pump` (what the app calls every frame), `faces.suggest {ids?, threshold?, margin?}`, `faces.rank {id?, index}` (every named person ranked by likeness to one face, read-only, never embeds), `faces.person {name, more?}` (a person's faces and the unnamed faces that look like them), `faces.unnamed {limit?}` (every unnamed face, the most confident suggestions first, then the look-alikes together, then the faces not embedded yet; with suggested names), `faces.setName {id?, index, name}`, `faces.nameFaces {faces: [{photo, index, name?}], name?}` (name many at once, a name each or one for all, one undo step), `faces.reject {faces: [{photo, index, name?}], name?, clearName?}` (reject a name for many faces, a name each or one for all, one undo step; `clearName` also clears the name of a face that is called the rejected name; replies `{rejected, cleared, skipped, photos}`, a face called the rejected name being left alone (counted in `skipped`) unless `clearName` is true; `faces.rank` marks each name a face rejected with `rejected: true`), `faces.removeBoxes {faces: [{photo, index}]}` (remove the face boxes of many faces, across photos and several on one photo, one undo step; replies `{removed, photos}`; boxes that are missing or are not faces are skipped), `faces.renamePerson {from, to, merge?}` (rename every in-library face of one person, or merge two when `merge` is true, one undo step) and `faces.evaluate`. `acknowledged` must be `true`: the caller has shown the user the terms and the user agreed. Installing makes the model the one in use and switches recognition on unless `activate` is `false`.

## Find Blemishes

Photo ▸ Find Blemishes (`spot.findBlemishes`) uses the installed detector's boxes and landmarks to look for small,
round, darker-or-redder spots on each face's skin and adds a heal spot on each, the way Find Dust Spots does for
sensor dust. Eyes, brows, nostrils and the mouth are cut out around the landmarks; faces narrower than 160 px in
the 1600 px analysis image get nothing, as do faces with more than 12 candidates (stubble, freckles and pores are
texture, not blemishes); at most six per face. It needs no model beyond the detector and never changes anything
without a spot you can see and delete (`crates/pipeline/src/blemish.rs`).
