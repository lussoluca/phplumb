<?php

namespace Symfony\Bridge\Twig\Mime;

class TemplatedEmail
{
    public function htmlTemplate(?string $template): static
    {
        return $this;
    }

    public function textTemplate(?string $template): static
    {
        return $this;
    }
}
