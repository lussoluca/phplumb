<?php

namespace App\Controller;

use Symfony\Component\Routing\Attribute\Route;

class LocalizedController
{
    #[Route(path: ['en' => '/about-us', 'it' => '/chi-siamo'], name: 'about_us')]
    public function about(): void
    {
    }

    #[Route('/aliased', name: 'aliased', alias: ['old_aliased'])]
    public function aliased(): void
    {
    }
}
