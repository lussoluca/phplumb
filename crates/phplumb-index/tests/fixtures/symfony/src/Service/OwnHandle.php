<?php

namespace App\Service;

use App\Message\SendSms;

final class OwnHandle
{
    public function ask(): void
    {
        $this->handle(new SendSms());
    }

    private function handle(object $message): void
    {
    }
}
