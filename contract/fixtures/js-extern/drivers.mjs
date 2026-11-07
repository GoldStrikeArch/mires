// The reference emissions, one per `#[js_extern]` call shape.
//
// Each driver is the JAVASCRIPT a correct lowering has to be equivalent to. It
// is executed against `chart-lib.mjs` and the resulting trace is recorded into
// `vectors.json`, so the vector's expected trace is not a transcription of what
// somebody thought the shape does -- it is what this expression actually did.
//
// The backend's own check runs its COMPILED module against the same fake and
// compares traces. It does not have to emit this text; it has to be
// indistinguishable from it at the trace.
//
// WHY EACH DRIVER CARRIES `assert`
// --------------------------------
// A recorded trace cannot disagree with the code that produced it, so recording
// alone proves nothing about INTENT: a driver that quietly stopped exercising
// the trailing-argument case would re-record happily and stay green forever.
// `assert` is the hand-written claim about the shape -- the part a diff cannot
// self-justify. It runs on every check, against the freshly recorded trace.
//
// SETUP IS NOT MEASURED. The recorder is reset between `setup` and `run`, so a
// vector about `update` contains the `update` and nothing else. Labels are
// computed at record time, so the instance built during setup still appears as
// `chart#0` on its first appearance in the measured phase.

/**
 * The inputs every driver shares. Plain data, so the vectors record argument
 * SHAPES rather than whatever a test happened to have lying around.
 *
 * `canvas` stands in for the DOM element a chart is mounted on. It is not a
 * real element and does not need to be: `chart-lib.mjs` normalizes anything
 * with a string `nodeName` to `node(<tag>)`, so a jsdom canvas and this record
 * identically, and the fake never touches the DOM.
 */
export const env = {
  canvas: { nodeName: "CANVAS", id: "revenue" },
  config: {
    type: "line",
    title: "Revenue",
    data: {
      labels: ["Q1", "Q2"],
      datasets: [
        { label: "Revenue", values: [10, 20] },
        { label: "Cost", values: [4, 6] }
      ],
      points: [
        { x: 0, y: 10 },
        { x: 1, y: 20 }
      ]
    }
  },
  nextData: {
    labels: ["Q3"],
    datasets: [{ label: "Revenue", values: [30] }],
    points: [{ x: 2, y: 30 }]
  },
  plugin: { id: "tooltip" }
};

/** Build an instance without recording it. Used as `setup` by most vectors. */
const chart = (m) => new m.Chart(env.canvas, env.config);

const only = (trace) => (trace.length === 1 ? trace[0] : {});

export const DRIVERS = [
  // ------------------------------------------------------------------- new
  {
    id: "new-named",
    shape: "new",
    required: true,
    why: "Construction through the NAMED export, the ordinary case: two arguments, a fresh instance returned.",
    run: (m) => new m.Chart(env.canvas, env.config),
    assert: ({ trace, returned }) => [
      ["exactly one operation is recorded", trace.length === 1],
      ["it is a `new`, not a call", only(trace).op === "new"],
      ["the named binding is the one used", only(trace).via === "named"],
      ["two arguments are passed", only(trace).argc === 2],
      [
        "the mount target normalizes to a canvas node",
        only(trace).args && only(trace).args[0] === "node(canvas)"
      ],
      ["the instance is the value returned", returned === "chart#0"]
    ]
  },
  {
    id: "new-default",
    shape: "new",
    required: true,
    why: "The same construction through the DEFAULT export. The library offers both bindings; a descriptor has to name one, and the trace records which was taken.",
    run: (m) => new m.default(env.canvas, env.config),
    assert: ({ trace }) => [
      ["it is a `new`", only(trace).op === "new"],
      ["the default binding is the one used", only(trace).via === "default"],
      [
        "the two bindings are otherwise indistinguishable at the trace",
        only(trace).ctor === "Chart" && only(trace).argc === 2
      ]
    ]
  },
  {
    id: "new-without-new",
    shape: "new",
    required: true,
    why: "The negative for the `new` shape. A lowering that emits a plain call where the descriptor said `new` does not silently produce a wrong-looking chart -- it throws, the way a real class does. Recorded so the failure mode is pinned rather than assumed.",
    run: (m) => m.Chart(env.canvas, env.config),
    assert: ({ trace, threw }) => [
      ["the attempt is recorded before the throw", only(trace).op === "new.withoutNew"],
      ["it throws a TypeError", threw !== null && threw.startsWith("TypeError")],
      [
        "the message names the missing `new`",
        threw !== null && threw.includes("without 'new'")
      ]
    ]
  },

  // ------------------------------------------------------------------ send
  {
    id: "send-one-arg",
    shape: "send",
    required: true,
    why: "A method on an instance, one argument, no useful return value. The commonest shape in the dashboard: push new data at an existing chart.",
    setup: chart,
    run: (m, instance) => instance.update(env.nextData),
    assert: ({ trace, returned }) => [
      ["exactly one operation is recorded", trace.length === 1],
      ["it is a `send`", only(trace).op === "send"],
      ["it is dispatched on the instance, not on the module", only(trace).target === "chart#0"],
      ["the method name is carried, not the binding name", only(trace).method === "update"],
      ["one argument", only(trace).argc === 1],
      ["the argument is the data object, structurally", typeof only(trace).args[0] === "object"],
      ["nothing is returned", returned === "undefined"]
    ]
  },
  {
    id: "send-zero-args",
    shape: "send",
    required: true,
    why: "A zero-argument method. The empty argument list has to be empty, not a list containing one `undefined` -- the same asymmetry PROCEDURES.md records for the procedure wire, in a different place.",
    setup: chart,
    run: (m, instance) => instance.destroy(),
    assert: ({ trace }) => [
      ["it is a `send`", only(trace).op === "send"],
      ["no arguments at all", only(trace).argc === 0],
      ["the argument list is empty, not [undefined]", only(trace).args.length === 0]
    ]
  },
  {
    id: "send-trailing-omitted",
    shape: "send",
    required: true,
    why: "An optional trailing argument the caller did not supply. This is Melange's trailing-`undefined` stripping made observable: the library branches on `arguments.length`, so the two spellings are NOT equivalent, and the descriptor has to pick one.",
    setup: chart,
    run: (m, instance) => instance.resize(800),
    assert: ({ trace }) => [
      ["one argument reaches the library", only(trace).argc === 1],
      ["and it is the one that was written", only(trace).args[0] === 800]
    ]
  },
  {
    id: "send-trailing-undefined",
    shape: "send",
    required: true,
    why: "The same call written with the trailing argument spelled `undefined`. Its trace differs from `send-trailing-omitted` in `argc`, which is the whole point: if a lowering pads optional arguments, this is where it shows.",
    setup: chart,
    run: (m, instance) => instance.resize(800, undefined),
    assert: ({ trace }) => [
      ["two arguments reach the library", only(trace).argc === 2],
      ["the second is recorded as undefined, not as null", only(trace).args[1] === "undefined"]
    ]
  },

  // --------------------------------------------------------------- nullable
  {
    id: "nullable-some",
    shape: "send",
    required: true,
    returnWrapper: "nullToOption",
    why: "The in-range read of a fallible accessor. Half of the `null_to_opt` return wrapper: this arm has to arrive as a present value.",
    setup: chart,
    run: (m, instance) => instance.getPoint(0),
    assert: ({ trace, returned }) => [
      ["a value comes back", only(trace).returned !== null],
      ["and it is the point object", returned && returned.x === 0 && returned.y === 10]
    ]
  },
  {
    id: "nullable-none",
    shape: "send",
    required: true,
    returnWrapper: "nullToOption",
    why: "The out-of-range read. The library answers `null` -- JSON's null, not `undefined`. A wrapper that only tests `=== undefined` passes every other vector on this page and fails here.",
    setup: chart,
    run: (m, instance) => instance.getPoint(99),
    assert: ({ trace, returned }) => [
      ["the library answers null", returned === null],
      ["recorded as null and not as the string \"undefined\"", only(trace).returned === null]
    ]
  },

  // -------------------------------------------------------------------- get
  {
    id: "get-property",
    shape: "get",
    required: true,
    why: "Reading a property off an instance. Distinct from `send`: no call, no arguments, and a lowering that emitted `chart.data()` would find a non-function.",
    setup: chart,
    run: (m, instance) => instance.data,
    assert: ({ trace, returned }) => [
      ["exactly one operation", trace.length === 1],
      ["it is a `get`", only(trace).op === "get"],
      ["on the instance", only(trace).target === "chart#0"],
      ["naming the property", only(trace).prop === "data"],
      ["the value is a further view, labelled by its path", returned === "chart#0.data"]
    ]
  },
  {
    id: "get-scoped",
    shape: "get",
    required: true,
    why: "A two-step read, `chart.data.datasets`. The descriptor's scope axis is what makes this one declaration rather than two, and the trace shows both steps -- a lowering cannot collapse them, because each step is a real property read.",
    setup: chart,
    run: (m, instance) => instance.data.datasets,
    assert: ({ trace }) => [
      ["both steps are recorded, in order", trace.length === 2],
      ["the first reads `data` off the instance", trace[0].prop === "data"],
      ["the second reads `datasets` off THAT view", trace[1].target === "chart#0.data"],
      ["the path is legible in the label", trace[1].value === "chart#0.data.datasets"]
    ]
  },
  {
    id: "get-static",
    shape: "get",
    required: true,
    why: "A property on the module binding rather than on an instance. Same shape, different scope root.",
    run: (m) => m.Chart.version,
    assert: ({ trace, returned }) => [
      ["it is a `get`", only(trace).op === "get"],
      ["rooted at the binding, not at an instance", only(trace).target === "Chart"],
      ["through the named export", only(trace).via === "named"],
      ["the value is the version string", returned === "0.0.0-fake"]
    ]
  },
  {
    id: "set-property",
    shape: "set",
    required: false,
    why: "Assignment to a property. Listed as not-required for this wave -- the dashboard's four shapes are new/send/get/index -- but it shares the getter's scope machinery and costs nothing to pin now.",
    setup: chart,
    run: (m, instance) => {
      instance.title = "Revenue, restated";
    },
    assert: ({ trace }) => [
      ["it is a `set` and not a `send`", only(trace).op === "set"],
      ["it names the property", only(trace).prop === "title"],
      ["it carries the assigned value", only(trace).value === "Revenue, restated"]
    ]
  },

  // ------------------------------------------------------------------ index
  {
    id: "index-in-range",
    shape: "index",
    required: true,
    why: "An element read out of an indexed collection. The index is a runtime value, which is why this is its own shape rather than another `get` with a constant path.",
    setup: chart,
    run: (m, instance) => instance.data.datasets[0],
    assert: ({ trace, returned }) => [
      ["three steps: two scope reads then the index", trace.length === 3],
      ["the last is an `index`", trace[2].op === "index"],
      ["it is numeric, not the string \"0\"", trace[2].index === 0],
      ["rooted at the collection", trace[2].target === "chart#0.data.datasets"],
      ["and it yields the first dataset", returned && returned.label === "Revenue"]
    ]
  },
  {
    id: "index-length",
    shape: "index",
    required: true,
    why: "The collection's length. Anything iterating the collection from Rust needs it, and it is a `get` on the collection rather than an `index` -- so a descriptor that models a collection as opaque cannot express the loop.",
    setup: chart,
    run: (m, instance) => instance.data.datasets.length,
    assert: ({ trace, returned }) => [
      ["length is a `get`, not an `index`", trace[2].op === "get"],
      ["it names the property", trace[2].prop === "length"],
      ["two datasets were configured", returned === 2]
    ]
  },
  {
    id: "index-out-of-range",
    shape: "index",
    required: true,
    why: "The out-of-range index. The collection answers `undefined`, where `getPoint` answers `null` for the same class of mistake. Both are real library behaviour and a descriptor's return wrapper has to say which one it accepts -- assuming they are interchangeable is the bug this vector exists to catch.",
    setup: chart,
    run: (m, instance) => instance.data.datasets[9],
    assert: ({ trace, returned }) => [
      ["the read is still recorded", trace[2].op === "index"],
      ["it yields undefined", returned === "undefined"],
      [
        "recorded as \"undefined\", NOT as null -- unlike nullable-none",
        trace[2].value === "undefined"
      ]
    ]
  },

  // ------------------------------------------------------------------- call
  {
    id: "call-static",
    shape: "call",
    required: false,
    why: "A static method on the binding: `Chart.register(plugin)`. Not one of this wave's four, but it is how the real library is initialised, so the dashboard will need it and the vector is here to be met rather than discovered.",
    run: (m) => m.Chart.register(env.plugin),
    assert: ({ trace }) => [
      ["it is a `call`, not a `send` and not a `new`", only(trace).op === "call"],
      ["rooted at the binding", only(trace).target === "Chart"],
      ["naming the method", only(trace).method === "register"],
      ["one argument", only(trace).argc === 1]
    ]
  }
];
