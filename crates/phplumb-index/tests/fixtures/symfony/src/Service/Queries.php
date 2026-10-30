<?php

namespace App\Service;

use App\Message\SendSms;
use Symfony\Component\Messenger\HandleTrait;

final class Queries
{
    use HandleTrait;

    public function ask(): void
    {
        $this->handle(new SendSms());
    }
}
