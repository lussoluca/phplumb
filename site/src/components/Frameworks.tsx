import { useId } from 'react'
import clsx from 'clsx'

import { Container } from '@/components/Container'

interface Group {
  name: string
  summary: string
  items: Array<string>
  icon: React.ComponentType
}

const groups: Array<Group> = [
  {
    name: 'Core PHP',
    summary: 'Every navigation request you make while reading code.',
    items: [
      'Go to definition through type inference and PHPDoc types',
      'Find implementations of interfaces, abstract classes and methods',
      'Find all references, through interfaces and parent methods',
      'Call hierarchy and type hierarchy',
      'Hover, workspace symbol search and file outline',
      'Diagnostics for classes, functions and methods that exist nowhere',
    ],
    icon: function CodeIcon() {
      return (
        <path
          d="m14 12-5 6 5 6M22 12l5 6-5 6"
          stroke="#fff"
          strokeWidth={2}
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      )
    },
  },
  {
    name: 'Drupal',
    summary: 'Code that calls almost everything through services.',
    items: [
      'Services typed from every *.services.yml',
      'Entity storage and entity classes from getStorage() and load()',
      'Hooks, from implementation to invocation and back',
      'Plugin ids, annotations and attributes',
      'Route names and services, routing and links YAML files',
      'Diagnostics for unknown service ids, routes and plugin ids',
    ],
    icon: function DrupalIcon() {
      let id = useId()
      return (
        <>
          <defs>
            <linearGradient
              id={id}
              x1="18"
              y1="8"
              x2="18"
              y2="28"
              gradientUnits="userSpaceOnUse"
            >
              <stop offset=".2" stopColor="#fff" />
              <stop offset={1} stopColor="#6692F1" />
            </linearGradient>
          </defs>
          <path
            d="M18 8c3 4 8 7 8 12a8 8 0 0 1-16 0c0-5 5-8 8-12Z"
            stroke={`url(#${id})`}
            strokeWidth={2}
            strokeLinejoin="round"
          />
        </>
      )
    },
  },
  {
    name: 'Symfony',
    summary: 'Routes, templates and messages wired by name.',
    items: [
      'Routes from attributes and YAML, also from Twig path() and url()',
      'Templates in render(), #[Template] and Twig tags',
      'Messenger, from dispatch() to every handler and back',
      'Parameters and service configuration in YAML',
      'Doctrine XML mappings and repositories',
      'Diagnostics for unknown routes, templates and parameters',
    ],
    icon: function SymfonyIcon() {
      return (
        <>
          <path
            opacity=".5"
            d="M9 12h18M9 24h18"
            stroke="#fff"
            strokeWidth={2}
            strokeLinecap="round"
          />
          <path
            d="M9 18h18"
            stroke="#fff"
            strokeWidth={2}
            strokeLinecap="round"
          />
        </>
      )
    },
  },
]

function CheckIcon({
  className,
  ...props
}: React.ComponentPropsWithoutRef<'svg'>) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 24 24"
      className={clsx('h-6 w-6 flex-none', className)}
      {...props}
    >
      <path
        d="m8 12.5 3 3 5-6.5"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.5}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  )
}

export function Frameworks() {
  return (
    <section
      id="frameworks"
      aria-label="Supported languages and frameworks"
      className="pt-20 pb-14 sm:pt-32 sm:pb-20 lg:pb-32"
    >
      <Container>
        <div className="mx-auto max-w-2xl md:text-center">
          <h2 className="font-display text-3xl tracking-tight text-slate-900 sm:text-4xl">
            Knows how your framework connects code.
          </h2>
          <p className="mt-4 text-lg tracking-tight text-slate-700">
            Drupal and Symfony turn on by themselves when the project’s
            composer.lock requires them. Each module answers only for its own
            project, so a Drupal codebase inside a Symfony application works
            too.
          </p>
        </div>
        <ul
          role="list"
          className="mx-auto mt-16 grid max-w-2xl grid-cols-1 gap-x-8 gap-y-12 lg:mt-20 lg:max-w-none lg:grid-cols-3"
        >
          {groups.map((group) => (
            <li
              key={group.name}
              className="rounded-4xl bg-slate-50 p-8 ring-1 ring-slate-900/5"
            >
              <div className="w-9 rounded-lg bg-blue-600">
                <svg aria-hidden="true" className="h-9 w-9" fill="none">
                  <group.icon />
                </svg>
              </div>
              <h3 className="mt-6 text-sm font-medium text-blue-600">
                {group.name}
              </h3>
              <p className="mt-2 font-display text-xl text-slate-900">
                {group.summary}
              </p>
              <ul role="list" className="mt-6 flex flex-col gap-y-3 text-sm">
                {group.items.map((item) => (
                  <li key={item} className="flex text-slate-700">
                    <CheckIcon className="text-blue-600" />
                    <span className="ml-3">{item}</span>
                  </li>
                ))}
              </ul>
            </li>
          ))}
        </ul>
        <p className="mx-auto mt-16 max-w-2xl text-center text-base text-slate-700">
          Agents edit files outside the editor, often many at once. PHPlumb
          follows changes on disk, including a branch switch, and caches the
          index so reopening a project skips files that did not change.
        </p>
      </Container>
    </section>
  )
}
