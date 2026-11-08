import FingerprintJS, {
  type Component,
  type Confidence,
  type GetResult,
  type LoadOptions,
  type Source,
  componentsToDebugString,
  hashComponents,
  load,
  loadSources,
  murmurX64Hash128,
  sources,
  transformSource,
} from '../../dist-types/fp'

const options: Readonly<LoadOptions> = { debug: false, delayFallback: 50, monitoring: true }
const agentPromise = load(options)
const defaultAgentPromise = FingerprintJS.load(options)
const confidence: Confidence = { score: 0.5, comment: 'fixture' }
const component: Component<string> = { value: 'value', duration: 1 }
const source: Source<{ suffix: string }, string> = ({ suffix }) => () => Promise.resolve(`value${suffix}`)
const transformed = transformSource(source, (value) => value.length)
const getCustom = loadSources({ source, transformed }, { suffix: '!' }, [] as const)

void agentPromise.then((agent) => agent.get()).then((result: GetResult) => {
  result.visitorId = hashComponents(result.components)
  console.log(result.version, result.confidence.score, componentsToDebugString(result.components))
})
void defaultAgentPromise
void confidence
void component
void getCustom()
void murmurX64Hash128('Unicode: \u{1f600}', 0)
void Object.keys(sources)
