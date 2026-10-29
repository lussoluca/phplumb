<?php

namespace App\Controller;

use Symfony\Component\Routing\Attribute\Route;

#[Route('/panel', name: null)]
class PanelController
{
    #[Route('/home', name: 'panel_home')]
    public function home(): void
    {
    }
}
