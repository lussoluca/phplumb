<?php

namespace App\Controller;

use Symfony\Component\Routing\Attribute\Route;

class LoginController
{
    public const ROUTE = 'app_login';

    #[Route('/login', name: self::ROUTE)]
    public function login(): void
    {
    }
}
