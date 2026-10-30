import Image from 'next/image'

import { Container } from '@/components/Container'
import backgroundImage from '@/images/background-faqs.jpg'
import { links } from '@/lib/links'

function Code({ children }: { children: React.ReactNode }) {
  return <code className="font-mono text-[0.8125rem]">{children}</code>
}

const faqs: Array<Array<{ question: string; answer: React.ReactNode }>> = [
  [
    {
      question: 'Should I keep Intelephense enabled?',
      answer:
        'In VS Code, disable Intelephense and other PHP navigation extensions in workspaces where you use PHPlumb, or you will see every result twice. In Zed, keep a server such as Phpactor or Intelephense for completion and list it first.',
    },
    {
      question: 'Does PHPlumb autocomplete or refactor code?',
      answer:
        'No. PHPlumb is for reading code. It has no autocomplete, signature help, rename or other refactorings, formatting, code actions or debugging.',
    },
    {
      question: 'Which platforms does it run on?',
      answer:
        'macOS on Apple silicon and Linux on x64, in VS Code 1.140 or later and in Zed.',
    },
  ],
  [
    {
      question: 'How does it detect Drupal and Symfony?',
      answer: (
        <>
          Drupal support turns on when a <Code>composer.lock</Code> in the
          project folder, or up to three folders below it, requires{' '}
          <Code>drupal/core</Code>. Symfony support looks for{' '}
          <Code>symfony/framework-bundle</Code>. A setting forces either on or
          off.
        </>
      ),
    },
    {
      question: 'Does it see files that an agent changes outside the editor?',
      answer:
        'Yes. PHPlumb follows changes on disk, including a branch switch, without you opening the files.',
    },
    {
      question: 'Can I index only part of the workspace?',
      answer: (
        <>
          Set <Code>phplumb.root</Code> to the folder of the PHP project, such
          as <Code>src/drupal</Code>, and <Code>phplumb.exclude</Code> to glob
          patterns of paths not to index.
        </>
      ),
    },
  ],
  [
    {
      question: 'What do the broken reference warnings report?',
      answer:
        'Classes, functions, methods and constants that exist nowhere, in project code only, so code an agent invented stands out. Dependencies such as vendor are skipped.',
    },
    {
      question: 'I see a warning on code that works. What now?',
      answer: (
        <>
          A warning on working code is a bug. Please{' '}
          <a
            href={links.issues}
            className="font-medium text-blue-600 hover:text-blue-500"
          >
            open an issue
          </a>{' '}
          with the code that triggers it.
        </>
      ),
    },
    {
      question: 'Is PHPlumb free?',
      answer: 'Yes. PHPlumb is open source under the MIT license.',
    },
  ],
]

export function Faqs() {
  return (
    <section
      id="faq"
      aria-labelledby="faq-title"
      className="relative overflow-hidden bg-slate-50 py-20 sm:py-32"
    >
      <Image
        className="absolute top-0 left-1/2 max-w-none translate-x-[-30%] -translate-y-1/4"
        src={backgroundImage}
        alt=""
        width={1558}
        height={946}
        unoptimized
      />
      <Container className="relative">
        <div className="mx-auto max-w-2xl lg:mx-0">
          <h2
            id="faq-title"
            className="font-display text-3xl tracking-tight text-slate-900 sm:text-4xl"
          >
            Frequently asked questions
          </h2>
          <p className="mt-4 text-lg tracking-tight text-slate-700">
            If you can’t find what you’re looking for, ask on{' '}
            <a
              href={links.issues}
              className="font-medium text-blue-600 hover:text-blue-500"
            >
              GitHub
            </a>
            .
          </p>
        </div>
        <ul
          role="list"
          className="mx-auto mt-16 grid max-w-2xl grid-cols-1 gap-8 lg:max-w-none lg:grid-cols-3"
        >
          {faqs.map((column, columnIndex) => (
            <li key={columnIndex}>
              <ul role="list" className="flex flex-col gap-y-8">
                {column.map((faq, faqIndex) => (
                  <li key={faqIndex}>
                    <h3 className="font-display text-lg/7 text-slate-900">
                      {faq.question}
                    </h3>
                    <p className="mt-4 text-sm text-slate-700">{faq.answer}</p>
                  </li>
                ))}
              </ul>
            </li>
          ))}
        </ul>
      </Container>
    </section>
  )
}
