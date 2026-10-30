<?php

namespace App\Controller;

use App\Message\SendEmail;
use App\Message\SendSms;
use App\Message\Unhandled;
use Symfony\Component\Messenger\Envelope;
use Symfony\Component\Messenger\MessageBusInterface;

class MessagesController
{
    public function __construct(private MessageBusInterface $bus)
    {
    }

    public function send($message): void
    {
        $this->bus->dispatch(new SendEmail());
        $this->bus->dispatch(new SendSms());
        $this->bus->dispatch(new Unhandled());
        $this->bus->dispatch($message);
        $this->bus->dispatch(new Envelope(new SendEmail()));
        $this->bus->dispatch(Envelope::wrap(new SendSms()));
        $this->bus->dispatch(message: new SendEmail());
        $envelope = new Envelope(new SendEmail());
        $this->bus->dispatch($envelope);
    }
}
