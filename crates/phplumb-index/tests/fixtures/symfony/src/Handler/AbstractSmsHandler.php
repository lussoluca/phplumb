<?php

namespace App\Handler;

use App\Message\SendSms;
use Symfony\Component\Messenger\Attribute\AsMessageHandler;

abstract class AbstractSmsHandler
{
    #[AsMessageHandler]
    public function onAbstractSms(SendSms $message): void
    {
    }
}
