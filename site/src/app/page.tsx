import { CallToAction } from '@/components/CallToAction'
import { Faqs } from '@/components/Faqs'
import { Footer } from '@/components/Footer'
import { Frameworks } from '@/components/Frameworks'
import { Header } from '@/components/Header'
import { Hero } from '@/components/Hero'
import { Install } from '@/components/Install'
import { PrimaryFeatures } from '@/components/PrimaryFeatures'
import { Speed } from '@/components/Speed'

export default function Home() {
  return (
    <>
      <Header />
      <main>
        <Hero />
        <PrimaryFeatures />
        <Frameworks />
        <CallToAction />
        <Speed />
        <Install />
        <Faqs />
      </main>
      <Footer />
    </>
  )
}
