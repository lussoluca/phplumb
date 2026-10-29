import { Container } from '@/components/Container'
import { links } from '@/lib/links'

const measurements = [
  { value: '2 to 4 s', task: 'Index the whole project from scratch' },
  { value: '1.7 s', task: 'Start again from the cache' },
  {
    value: '100 to 135 ms',
    task: 'Find 1,033 references of EntityStorageInterface::load',
  },
  { value: 'about 30 ms', task: 'Search symbols across the workspace' },
]

export function Speed() {
  return (
    <section
      id="speed"
      aria-labelledby="speed-title"
      className="bg-slate-50 py-20 sm:py-32"
    >
      <Container>
        <div className="mx-auto max-w-2xl md:text-center">
          <h2
            id="speed-title"
            className="font-display text-3xl tracking-tight text-slate-900 sm:text-4xl"
          >
            Fast on 40,000 files.
          </h2>
          <p className="mt-4 text-lg tracking-tight text-slate-700">
            Measured on a Drupal 11 project with 39,417 PHP files, on an Apple
            M3 Pro. Peak memory stays under 500 MB.
          </p>
        </div>
        <dl className="mx-auto mt-16 grid max-w-2xl grid-cols-1 gap-6 sm:grid-cols-2 lg:mt-20 lg:max-w-none lg:grid-cols-4">
          {measurements.map((measurement) => (
            <div
              key={measurement.task}
              className="flex flex-col rounded-2xl bg-white p-6 shadow-xl shadow-slate-900/10"
            >
              <dt className="order-last mt-2 text-sm text-slate-700">
                {measurement.task}
              </dt>
              <dd className="font-display text-3xl tracking-tight text-blue-600">
                {measurement.value}
              </dd>
            </div>
          ))}
        </dl>
        <p className="mt-10 text-center text-sm text-slate-700">
          See{' '}
          <a
            href={links.benchmarks}
            className="font-medium text-blue-600 hover:text-blue-500"
          >
            the benchmark history
          </a>{' '}
          for every run.
        </p>
      </Container>
    </section>
  )
}
