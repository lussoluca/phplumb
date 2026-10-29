import Link from 'next/link'

import { Button } from '@/components/Button'
import { Container } from '@/components/Container'
import { Logo } from '@/components/Logo'

export default function NotFound() {
  return (
    <Container className="flex flex-auto flex-col items-center justify-center py-20 text-center">
      <Link href="/" aria-label="Home">
        <Logo className="h-10 w-fit" />
      </Link>
      <p className="mt-20 text-sm font-medium text-slate-700">404</p>
      <h1 className="mt-3 text-lg font-semibold text-slate-900">
        Page not found
      </h1>
      <p className="mt-3 text-sm text-slate-700">
        Sorry, we couldn’t find the page you’re looking for.
      </p>
      <Button href="/" className="mt-10">
        Go back home
      </Button>
    </Container>
  )
}
