<?php

namespace App\Handler;

use App\Message\SendEmail;

trait HandlesEmail
{
    public function __invoke(SendEmail $message): void
    {
    }
}
